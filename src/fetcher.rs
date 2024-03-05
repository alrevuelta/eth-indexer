use ethers_core::types::U256;
use ethers_core::types::U64;
use ethers_providers::Middleware;
use ethers_providers::{Http, Provider};
use hex::encode;
use lighthouse::{types::BlockId, types::StateId, BeaconNodeHttpClient, SensitiveUrl, Timeouts};
use serde::{Deserialize, Serialize};
use std::cmp;
use std::convert::TryFrom;
use std::sync::Arc;
use std::time::{Duration, Instant};
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
use ethers_core::types::BlockId as BlockId2;
use ethers_core::types::BlockNumber;
use futures::future::try_join_all;

// See: https://flashbots.github.io/relay-specs/#/Data/getDeliveredPayloads
//
#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct DeliveredPayloadsResponse {
    pub slot: String,
    pub parent_hash: String,
    pub block_hash: String,
    pub builder_pubkey: String,
    pub proposer_pubkey: String,
    pub proposer_fee_recipient: String,
    pub gas_limit: String,
    pub gas_used: String,
    pub value: String,
    pub block_number: String,
    pub num_tx: String,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct RelayResponse {
    pub delivered_payloads_response: DeliveredPayloadsResponse,
    pub relay: String,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct BlockReward {
    pub block_number: u64,
    pub proposer_reward: String,
    pub fee_recipient: String,
    pub mev_reward: String,
    pub relay_responses: Vec<RelayResponse>,
}

fn sum_and_saturate(a: U256, b: U256, sat: U256) -> U256 {
    let res = a + b;
    if res > sat {
        return sat;
    }
    return res;
}

const RETRY_WAIT_SECONDS: u64 = 15;
const RELAY_URLS: [&str; 10] = [
    "https://0xa15b52576bcbf1072f4a011c0f99f9fb6c66f3e1ff321f11f461d15e31b1cb359caa092c71bbded0bae5b5ea401aab7e@aestus.live",
    "https://0xa7ab7a996c8584251c8f925da3170bdfd6ebc75d50f5ddc4050a6fdc77f2a3b5fce2cc750d0865e05d7228af97d69561@agnostic-relay.net",
    "https://0x8b5d2e73e2a3a55c6c87b8b6eb92e0149a125c852751db1422fa951e42a09b82c142c3ea98d0d9930b056a3bc9896b8f@bloxroute.max-profit.blxrbdn.com",
    "https://0xb0b07cd0abef743db4260b0ed50619cf6ad4d82064cb4fbec9d3ec530f7c5e6793d9f286c4e082c0244ffb9f2658fe88@bloxroute.regulated.blxrbdn.com",
    "https://0xb3ee7afcf27f1f1259ac1787876318c6584ee353097a50ed84f51a1f21a323b3736f271a895c7ce918c038e4265918be@relay.edennetwork.io",
    "https://0xac6e77dfe25ecd6110b8e780608cce0dab71fdd5ebea22a16c0205200f2f8e2e3ad3b71d3499c54ad14d6c21b41a37ae@boost-relay.flashbots.net",
    "https://0x98650451ba02064f7b000f5768cf0cf4d4e492317d82871bdc87ef841a0743f69f0f1eea11168503240ac35d101c9135@mainnet-relay.securerpc.com",
    "https://0xa1559ace749633b997cb3fdacffb890aeebdb0f5a3b6aaa7eeeaf1a38af0a8fe88b9e4b1f61f236d2e64d95733327a62@relay.ultrasound.money",
    "https://0x98650451ba02064f7b000f5768cf0cf4d4e492317d82871bdc87ef841a0743f69f0f1eea11168503240ac35d101c9135@mainnet-relay.securerpc.com",
    "https://0x8c7d33605ecef85403f8b7289c8058f440cbb6bf72b055dfe2f3e2c6695b6a1ea5a9cd0eb3a7982927a463feb4c3dae2@relay.wenmerge.com",
];

pub async fn get_mev_reward(block_number: u64) -> Vec<RelayResponse> {
    let mut tasks = Vec::new();

    for relay_url in RELAY_URLS.iter() {
        let task_n = tokio::spawn(async move {
            return get_mev_from_relay(block_number, relay_url.to_string()).await;
        });
        tasks.push(task_n);
    }
    let results: Result<Vec<Option<RelayResponse>>, tokio::task::JoinError> =
        try_join_all(tasks).await;

    return results
        .unwrap()
        .iter()
        .filter(|x| x.is_some())
        .map(|x| x.clone().unwrap())
        .collect();
}

pub async fn get_all_block_rewards(block_number: u64) -> BlockReward {
    let relay_responses = get_mev_reward(block_number).await;

    relay_responses.iter().all(|x| {
        x.delivered_payloads_response.value == relay_responses[0].delivered_payloads_response.value
    });

    let tip_feerec = get_vanila_reward(block_number).await;

    return BlockReward {
        block_number: block_number,
        proposer_reward: tip_feerec.0,
        fee_recipient: tip_feerec.1,
        mev_reward: relay_responses
            .iter()
            .map(|x| x.delivered_payloads_response.value.clone())
            .collect::<Vec<String>>()
            .get(0)
            .unwrap_or(&String::new())
            .to_string(),
        relay_responses: relay_responses,
    };
}

pub async fn get_mev_from_relay(block_number: u64, relay_url: String) -> Option<RelayResponse> {
    let relay_name = relay_url.split("@").collect::<Vec<&str>>()[1].to_string();
    // Infinite retries
    loop {
        let endpoint = format!(
            "{}/relay/v1/data/bidtraces/proposer_payload_delivered?block_number={}",
            relay_url, block_number
        );
        let response = reqwest::get(&endpoint).await;

        // Check if the request was successful (status code 2xx)
        if let Ok(response) = response {
            if response.status().is_success() {
                let json_data: Vec<DeliveredPayloadsResponse> = response.json().await.unwrap();

                match json_data.len() {
                    0 => {
                        //println!("No data for block: {:?}", block_number);
                        return None;
                    }
                    1 => {
                        //println!(
                        //    "Block: {:?} MEV Reward: {:?}",
                        //    block_number, json_data[0].value
                        //);
                        return Some(RelayResponse {
                            delivered_payloads_response: json_data[0].clone(),
                            relay: relay_name,
                        });
                    }
                    _ => {
                        println!("More than one entry for block: {:?}", block_number);
                        return None;
                    }
                }
            }
        } else {
            println!(
                "Error: {:?} with relay: {:?} retrying",
                response, relay_name
            );
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(RETRY_WAIT_SECONDS)).await;
    }
}

// Returns tip and fee recipient
pub async fn get_vanila_reward(block_number: u64) -> (String, String) {
    // TODO: store timestamp.
    // TODO add infinite retry logic: https://www.gakonst.com/ethers-rs/providers/retry.html
    let rpc_url = "http://localhost:8545";
    let provider = Arc::new(Provider::try_from(rpc_url).unwrap());

    // TODO: i need to replace this by a branch with my changes.
    let by_num = BlockId2::Number(BlockNumber::Number(block_number.into()));
    let block = provider.get_block_with_txs(by_num).await.unwrap();
    // receipts are neede since to get the gas used.
    let block_receipts = provider
        .get_block_receipts(BlockNumber::Number(block_number.into()))
        .await
        .unwrap();

    let base_fee_per_gas = block.as_ref().unwrap().base_fee_per_gas.unwrap();
    let burnt = block.as_ref().unwrap().gas_used * base_fee_per_gas;
    let got_block_number = block.as_ref().unwrap().number.unwrap();
    let fee_rec = block.as_ref().unwrap().author.unwrap();
    let mut tips = U256::from(0);

    for (idx, tx) in block.unwrap().transactions.iter().enumerate() {
        let gas_price = tx.gas_price.unwrap();
        let gas = tx.gas;

        // TODO: unsure how safe its indexing like this.
        let tx_receipt = &block_receipts[idx]; // TODO usur why &
        if tx_receipt.transaction_hash != tx.hash {
            panic!("Transaction hash mismatch");
        }

        //let what isthis = tx.gas; // not sure its the gas used
        let mut tipfee: U256 = U256::from(0);

        match tx.transaction_type {
            // EIP-2930 (0x01)
            Some(x) if x == U64::from(1) => {
                tipfee = gas_price * tx_receipt.gas_used.unwrap();
                /*println!(
                    "enter in 01: gasprice{:?} gas_used{:?} tipfee{:?}",
                    gas_price, gas, tipfee
                );*/
            }
            // EIP-1559 (0x02)
            Some(x) if x == U64::from(2) => {
                let gas_fee_cap = tx.max_fee_per_gas.unwrap();
                let gas_tip_cap = tx.max_priority_fee_per_gas.unwrap();
                let used_gas_price = sum_and_saturate(gas_tip_cap, base_fee_per_gas, gas_fee_cap);
                tipfee = used_gas_price * tx_receipt.gas_used.unwrap();
            }
            // TODO: new tx type in deneb 0x03
            // TODO: The matching here is unsafe and dirty. The order matters. Theoretically it enters here
            // for Legacy and Unkown transaction types.
            // Legacy (0x00)
            _ => {
                tipfee = gas_price * tx_receipt.gas_used.unwrap();
                /*
                println!(
                    "enter in 01: gasprice{:?} gas{:?} tipfee{:?}",
                    gas_price, gas, tipfee
                ); */
            }
        }
        tips = tips + tipfee;
    }
    let proposer_reward = tips - burnt;
    let fee_rec = format!("0x{}", encode(fee_rec.as_bytes()));
    return (proposer_reward.to_string(), fee_rec);
}
