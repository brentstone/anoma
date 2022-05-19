use anoma::ledger::eth_bridge::storage;
use anoma::proto::Signed;
use anoma::types::transaction;
use anoma::types::transaction::eth_bridge::{
    TransferFromEthereum, UpdateQueue,
};
use borsh::BorshDeserialize;

use crate::imports::tx;
use crate::tx_prelude::log_string;

const TX_NAME: &str = "tx_update_queue";

fn log(msg: &str) {
    log_string(format!("[{}] {}", TX_NAME, msg))
}

fn fatal(msg: &str, err: impl std::error::Error) -> ! {
    log(&format!("ERROR: {} - {:?}", msg, err));
    panic!()
}

fn fatal_msg(msg: &str) -> ! {
    log(msg);
    panic!()
}

pub fn update_queue(data: UpdateQueue) {
    log(&format!(
        "update_queue tx being executed ({} messages to enqueue)",
        data.enqueue.len()
    ));
    let queue_key = storage::queue_key();
    let mut queue: Vec<TransferFromEthereum> =
        if tx::has_key(queue_key.to_string()) {
            log("queue key exists");
            match tx::read(queue_key.to_string()) {
                Some(queue) => queue,
                None => fatal_msg("thought queue existed but it didn't"),
            }
        } else {
            log("initializing queue for the first time");
            tx::write(
                queue_key.to_string(),
                Vec::<TransferFromEthereum>::new(),
            );
            vec![]
        };
    log(&format!("got existing queue: {:#?}", queue));
    // TODO: dequeue and mint wrapped Ethereum assets first before pushing
    // new transfers
    for transfer in data.enqueue {
        queue.push(transfer);
    }
    tx::write(queue_key.to_string(), queue);
}

pub fn apply_tx(tx_data: Vec<u8>) {
    log(&format!("called with tx_data - {} bytes", tx_data.len()));
    let signed: Signed<Vec<u8>> = match Signed::try_from_slice(&tx_data) {
        Ok(signed) => {
            log("deserialized Signed<Vec<u8>>");
            signed
        }
        Err(error) => fatal("deserializing Signed<Vec<u8>>", error),
    };
    if signed.data.is_empty() {
        fatal_msg("data is empty")
    }
    log(&format!("got data - {} bytes", signed.data.len()));
    // we don't verify the signature here - the VP should do that

    log("attempting to update Ethereum bridge queue");
    let strct = match transaction::eth_bridge::UpdateQueue::try_from_slice(
        &signed.data,
    ) {
        Ok(strct) => {
            log(&format!("serialized data to: {:#?}", strct));
            strct
        }
        Err(error) => fatal("serializing data to UpdateQueue", error),
    };
    update_queue(strct);
}
