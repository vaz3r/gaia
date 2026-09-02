use crate::dht::node_id::random_node_id;
use crate::dht::routing_table::NodeInfo;
use crate::krpc::message::Message;
use crate::router::Router;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

pub async fn run_bep51_worker(
    router: Arc<Router>,
    interval: Duration,
    fresh_verify_tx: mpsc::Sender<[u8; 20]>,
) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut set = tokio::task::JoinSet::new();

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let nodes = router.random_routing_nodes(10);
                for node in nodes {
                    let r = router.clone();
                    let tx = fresh_verify_tx.clone();
                    set.spawn(async move {
                        if let Ok(infohashes) = send_sample_infohashes(&r, node).await {
                            for ih in infohashes {
                                let _ = tx.try_send(ih);
                            }
                        }
                    });
                }
            }
            Some(_) = set.join_next() => {
                // Reap completed tasks
            }
        }
    }
}

async fn send_sample_infohashes(router: &Arc<Router>, node: NodeInfo) -> Result<Vec<[u8; 20]>, ()> {
    let (txid, rx) = router.register(crate::krpc::message::SAMPLE_INFOHASHES);
    let sender_id = router.random_sybil_id();
    let target = random_node_id();

    let mut buf = [0u8; 256];
    let mut pos = 0;

    let b1 = b"d1:ad2:id20:";
    buf[pos..pos + b1.len()].copy_from_slice(b1);
    pos += b1.len();
    buf[pos..pos + 20].copy_from_slice(&sender_id);
    pos += 20;

    let b2 = b"6:target20:";
    buf[pos..pos + b2.len()].copy_from_slice(b2);
    pos += b2.len();
    buf[pos..pos + 20].copy_from_slice(&target);
    pos += 20;

    let b3 = format!("e1:q17:sample_infohashes1:t{}:", txid.len());
    buf[pos..pos + b3.len()].copy_from_slice(b3.as_bytes());
    pos += b3.len();
    buf[pos..pos + txid.len()].copy_from_slice(&txid);
    pos += txid.len();

    let b4 = b"1:y1:qe";
    buf[pos..pos + b4.len()].copy_from_slice(b4);
    pos += b4.len();

    router.try_send(&buf[..pos], node.addr);

    match tokio::time::timeout(Duration::from_secs(5), rx).await {
        Ok(Ok(bytes)) => {
            if let Ok(msg) = Message::parse(&bytes)
                && let crate::krpc::message::Kind::Response { r } = msg.kind
                && let Some(samples) = r.get_bytes(b"samples")
            {
                let mut infohashes = Vec::new();
                let mut i = 0;
                while i + 20 <= samples.len() {
                    let mut ih = [0u8; 20];
                    ih.copy_from_slice(&samples[i..i + 20]);
                    infohashes.push(ih);
                    i += 20;
                }
                return Ok(infohashes);
            }
            Err(())
        }
        _ => Err(()),
    }
}
