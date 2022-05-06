//! A tx for a PoS bond that stakes tokens via a self-bond or delegation.

use anoma_tx_prelude::proof_of_stake::bond_tokens;
use anoma_tx_prelude::*;

#[transaction]
fn apply_tx(tx_data: Vec<u8>) {
    let signed = SignedTxData::try_from_slice(&tx_data[..]).unwrap();
    let bond =
        transaction::pos::Bond::try_from_slice(&signed.data.unwrap()[..])
            .unwrap();

    if let Err(err) =
        bond_tokens(bond.source.as_ref(), &bond.validator, bond.amount)
    {
        debug_log!("Bond failed with: {}", err);
        panic!()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use anoma::proto::Tx;
    use anoma_tests::log::test;
    use anoma_tests::tx::*;
    use anoma_tx_prelude::address::testing::{
        arb_established_address, arb_non_internal_address,
    };
    use anoma_tx_prelude::address::InternalAddress;
    use anoma_tx_prelude::key::testing::arb_common_keypair;
    use anoma_tx_prelude::key::RefTo;
    use anoma_tx_prelude::proof_of_stake::parameters::testing::arb_pos_params;
    use anoma_tx_prelude::token;
    use anoma_vp_prelude::proof_of_stake::anoma_proof_of_stake::PosBase;
    use anoma_vp_prelude::proof_of_stake::types::Bond;
    use anoma_vp_prelude::proof_of_stake::{
        staking_token_address, BondId, GenesisValidator,
    };
    use proptest::prelude::*;

    use super::*;

    proptest! {
        /// An example test, checking that this transaction performs no storage
        /// modifications.
        #[test]
        fn test_no_op_transaction(
            (initial_stake, bond) in arb_initial_stake_and_bond(),
            key in arb_common_keypair(),
            pos_params in arb_pos_params()) {
            // The environment must be initialized first
            tx_host_env::init();


            tx_host_env::with(|tx_env| {
                let staking_reward_address = address::testing::established_address_1();
                let consensus_key = key::testing::keypair_1().ref_to();
                let staking_reward_key = key::testing::keypair_2().ref_to();

                // Ensure that all the used addresses exist
                tx_env.spawn_accounts([&bond.validator, &staking_reward_address, &staking_token_address()]);
                if let Some(source) = &bond.source {
                    tx_env.spawn_accounts([source]);
                }

                // Ensure that the bond's source has enough tokens for the bond
                let target = bond.source.as_ref().unwrap_or(&bond.validator);
                tx_env.credit_tokens(target, &staking_token_address(), bond.amount);

                let genesis_validator = &GenesisValidator {
                    address: bond.validator.clone(),
                    staking_reward_address,
                    tokens: initial_stake,
                    consensus_key,
                    staking_reward_key,
                };

                // Initialize PoS storage
                tx_env.storage
                    .init_genesis(
                        &pos_params,
                        [genesis_validator].into_iter(),
                        // TODO different starting epoch?
                        0,
                    )
                    .unwrap();
            });

            let tx_code = vec![];
            let tx_data = bond.try_to_vec().unwrap();
            let tx = Tx::new(tx_code, Some(tx_data));
            let signed_tx = tx.sign(&key);
            let tx_data = signed_tx.data.unwrap();

            // Read the data before the tx is executed
            let pos_balance_key = token::balance_key(&staking_token_address(), &Address::Internal(InternalAddress::PoS));
            let pos_balance_pre: token::Amount = read(&pos_balance_key.to_string()).expect("PoS must have balance");
            assert_eq!(pos_balance_pre, initial_stake);
            // let bonds_pre = PoS.read_bond(&bond.validator);

            apply_tx(tx_data);

            // Read the data after the tx is executed

            // The following storage keys should be updated:

            //     - `#{PoS}/validator/#{validator}/total_deltas`
            let total_delta_post = PoS.read_validator_total_deltas(&bond.validator);
            for epoch in 0..pos_params.pipeline_len {
                assert_eq!(total_delta_post.as_ref().unwrap().get(epoch), Some(initial_stake.into()), "The total deltas before the pipeline offset must not change - checking in epoch: {epoch}");
            }
            for epoch in pos_params.pipeline_len..=pos_params.unbonding_len {
                let expected_stake = i128::from(initial_stake) + i128::from(bond.amount);
                assert_eq!(total_delta_post.as_ref().unwrap().get(epoch), Some(expected_stake), "The total deltas at and after the pipeline offset epoch must be incremented by the bonded amount - checking in epoch: {epoch}");
            }

            //     - `#{staking_token}/balance/#{PoS}`
            let pos_balance_post: token::Amount = read(&pos_balance_key.to_string()).unwrap();
            assert_eq!(pos_balance_pre + bond.amount, pos_balance_post);

            //     - `#{PoS}/bond/#{owner}/#{validator}`
            let bond_src = bond.source.clone().unwrap_or_else(|| bond.validator.clone());
            let bond_id = BondId {validator: bond.validator.clone(), source: bond_src};
            let bonds_post = PoS.read_bond(&bond_id).unwrap();
            match &bond.source {
                Some(_) => {
                    // This bond was a delegation
                    for epoch in 0..pos_params.pipeline_len {
                        let bond: Option<Bond<token::Amount>> = bonds_post.get(epoch);
                        assert!(bond.is_none(), "Delegation before pipeline offset should be empty - checking epoch {epoch}");
                    }
                    for epoch in pos_params.pipeline_len..=pos_params.unbonding_len {
                        let start_epoch = anoma_tx_prelude::proof_of_stake::types::Epoch::from(pos_params.pipeline_len);
                        let expected_bond = HashMap::from_iter([(start_epoch, bond.amount)]);
                        let bond: Bond<token::Amount> = bonds_post.get(epoch).unwrap();
                        assert_eq!(bond.deltas, expected_bond, "Delegation at and after pipeline offset should be equal to the bonded amount - checking epoch {epoch}");
                    }
                },
                None => {
                    let genesis_epoch = anoma_tx_prelude::proof_of_stake::types::Epoch::from(0);
                    // It was a self-bond
                    for epoch in 0..pos_params.pipeline_len {
                        let expected_bond = HashMap::from_iter([(genesis_epoch, initial_stake)]);
                        let bond: Bond<token::Amount> = bonds_post.get(epoch).expect("Genesis validator should already have self-bond");
                        assert_eq!(bond.deltas, expected_bond, "Delegation before pipeline offset should be equal to the genesis initial stake - checking epoch {epoch}");
                    }
                    for epoch in pos_params.pipeline_len..=pos_params.unbonding_len {
                        let start_epoch = anoma_tx_prelude::proof_of_stake::types::Epoch::from(pos_params.pipeline_len);
                        let expected_bond = HashMap::from_iter([(genesis_epoch, initial_stake), (start_epoch, bond.amount)]);
                        let bond: Bond<token::Amount> = bonds_post.get(epoch).unwrap();
                        assert_eq!(bond.deltas, expected_bond, "Delegation at and after pipeline offset should contain genesis stake and the bonded amount - checking epoch {epoch}");
                    }
                }
            }

            // TODO:

            //     - `#{PoS}/total_voting_power` (optional)
            //     - `#{PoS}/validator_set` (optional)
            //     - `#{PoS}/validator/#{validator}/voting_power` (optional)

        }
    }

    prop_compose! {
        /// Generates an initial validator stake and a bond, while making sure
        /// that the `initial_stake + bond.amount <= u64::MAX` to avoid
        /// overflow.
        fn arb_initial_stake_and_bond()
            // Generate initial stake
            (initial_stake in token::testing::arb_amount())
            // Use the initial stake to limit the bond amount
            (bond in arb_bond(u64::MAX - u64::from(initial_stake)),
            // Use the generated initial stake too too
            initial_stake in Just(initial_stake),
        ) -> (token::Amount, transaction::pos::Bond) {
            (initial_stake, bond)
        }
    }

    fn arb_bond(
        max_amount: u64,
    ) -> impl Strategy<Value = transaction::pos::Bond> {
        (
            arb_established_address(),
            prop::option::of(arb_non_internal_address()),
            token::testing::arb_amount_ceiled(max_amount.into()),
        )
            .prop_map(|(validator, source, amount)| {
                transaction::pos::Bond {
                    validator: Address::Established(validator),
                    amount,
                    source,
                }
            })
    }
}
