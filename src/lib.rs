#[macro_use]
extern crate log;

use std::future::Future;
use std::io::Result;
use std::net::SocketAddr;
use std::ops::Deref;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use arc_swap::{ArcSwap, Cache};
use chrono::Utc;

pub trait UdpProxiSender {
    fn send<'a>(
        &'a self,
        packet: &'a [u8],
        from: SocketAddr,
        to: SocketAddr,
    ) -> impl Future<Output=Result<()>> + 'a + Send;
}

pub trait UdpProxiReceiver {
    fn recv<'a>(
        &'a self,
        buff: &'a mut [u8],
    ) -> impl Future<Output=Result<(usize, SocketAddr)>> + 'a + Send;
}

pub trait UdpProxiEndpointCreator {
    type TxRx;

    fn new_endpoint(
        &mut self,
        from: SocketAddr,
        to: SocketAddr,
    ) -> impl Future<Output=Result<Self::TxRx>>;
}

type MappingInner<Tx> = Vec<Arc<(SocketAddr, Tx, AtomicI64)>>;

pub struct UdpProxi<SrcTx, EndpointCreator>
where
    EndpointCreator: UdpProxiEndpointCreator,
{
    to_src: SrcTx,
    endpoint_creator: EndpointCreator,
    mapping: Arc<ArcSwap<MappingInner<EndpointCreator::TxRx>>>,
    mapping_cache: Cache<Arc<ArcSwap<MappingInner<EndpointCreator::TxRx>>>, Arc<MappingInner<EndpointCreator::TxRx>>>,
}

impl<SrcTx, EndpointCreator> UdpProxi<SrcTx, EndpointCreator>
where
    SrcTx: UdpProxiSender + Clone + Send + Sync + 'static,
    EndpointCreator: UdpProxiEndpointCreator,
    EndpointCreator::TxRx: UdpProxiSender + UdpProxiReceiver + Send + Sync + 'static,
{
    pub fn new(
        to_src: SrcTx,
        new_endpoint: EndpointCreator,
    ) -> Self {
        let mapping = Arc::new(ArcSwap::from_pointee(Vec::new()));

        UdpProxi {
            to_src,
            endpoint_creator: new_endpoint,
            mapping: mapping.clone(),
            mapping_cache: Cache::new(mapping),
        }
    }

    pub async fn send_packet(
        &mut self,
        packet: &[u8],
        from: SocketAddr,
        to: SocketAddr,
    ) -> Result<()> {
        let to_src = &self.to_src;
        let mapping = &self.mapping;
        let mapping_cache = &mut self.mapping_cache;
        let endpoint_creator = &mut self.endpoint_creator;
        let snap = mapping_cache.load();

        let item = snap
            .binary_search_by_key(&from, |v| (**v).0)
            .ok()
            .map(|i| &*snap.deref()[i]);

        let insert_item;

        let (_, to_socket, update_time) = match item {
            None => {
                let to_socket = endpoint_creator.new_endpoint(from, to).await?;

                insert_item = Arc::new((from, to_socket, AtomicI64::new(Utc::now().timestamp())));

                mapping.rcu(|v| {
                    let mut tmp = (**v).clone();

                    match tmp.binary_search_by_key(&from, |v| (**v).0) {
                        Ok(_) => unreachable!(),
                        Err(i) => tmp.insert(i, insert_item.clone()),
                    }
                    tmp
                });

                tokio::spawn({
                    let tx = to_src.clone();
                    let mapping = mapping.clone();
                    let insert_item = insert_item.clone();

                    async move {
                        let (_, to_socket, update_time) = &*insert_item;
                        let mut buff = vec![0u8; 65536];

                        let fut1 = async {
                            loop {
                                let (len, peer) = to_socket.recv(&mut buff).await?;
                                tx.send(&buff[..len], peer, from).await?;
                                update_time.store(Utc::now().timestamp(), Ordering::Relaxed);
                            }
                        };

                        let fut2 = async {
                            loop {
                                tokio::time::sleep(Duration::from_secs(5)).await;

                                if Utc::now().timestamp() - update_time.load(Ordering::Relaxed) >= 300 {
                                    return;
                                }
                            }
                        };

                        let res: Result<()> = tokio::select! {
                            res = fut1 => res,
                            _ = fut2 => Ok(())
                        };

                        if let Err(e) = res {
                            error!("child udp handler error: {}", e);
                        }

                        mapping.rcu(|v| {
                            let mut tmp = (**v).clone();

                            match tmp.binary_search_by_key(&from, |v| (**v).0) {
                                Ok(i) => tmp.remove(i),
                                Err(_) => unreachable!(),
                            };
                            tmp
                        });
                    }
                });

                &*insert_item
            }
            Some(v) => v,
        };

        to_socket.send(&packet, from, to).await?;
        update_time.store(Utc::now().timestamp(), Ordering::Relaxed);

        Ok(())
    }
}