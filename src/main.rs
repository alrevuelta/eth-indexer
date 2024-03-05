use eth_indexer::fetcher::get_all_block_rewards;
use eth_indexer::fetcher::BlockReward;
use futures::future::join_all;
use tokio::task::JoinError;

#[tokio::main]
async fn main() {
    let blocks_paralel: usize = 5;
    let from_block = 16308189u64;
    let to_block = 18908894;

    println!(
        "Processing blocks from {} to {} total: {}",
        from_block,
        to_block,
        to_block - from_block
    );

    let blocks_to_process_filter: Vec<u64> = (from_block..=to_block).collect();

    for chunk in blocks_to_process_filter.chunks(blocks_paralel) {
        let mut tasks = Vec::new();

        for &i in chunk {
            let task_n = tokio::spawn(async move {
                return get_all_block_rewards(i).await;
            });
            tasks.push(task_n);
        }

        let mut results: Vec<Result<BlockReward, JoinError>> = join_all(tasks).await;

        results.sort_by(|a, b| {
            let a = a.as_ref().unwrap().block_number;
            let b = b.as_ref().unwrap().block_number;
            a.cmp(&b)
        });

        for res in results {
            let res = res.unwrap();
            println!(
                "block={:?} tip={:?} fee_recipient={:?} mev={:?} mev_fee_recipient={:?} relays={:?}",
                res.block_number,
                res.proposer_reward,
                res.fee_recipient,
                res.mev_reward,
                res.relay_responses
                    .iter()
                    .map(|x| x.delivered_payloads_response.proposer_fee_recipient.clone())
                    .collect::<Vec<String>>()
                    .get(0)
                    .unwrap_or(&String::new())
                    .to_string(),
                res.relay_responses
                    .iter()
                    .map(|x| x.relay.clone())
                    .collect::<Vec<String>>()
            );
        }
    }
}
