use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rand::distributions::Bernoulli;
use rand::prelude::{Distribution, SmallRng};
use rand::{thread_rng, SeedableRng};
use tokio::sync::RwLock;

use crate::transport::{Socket, Transport, TransportError};
use crate::ChitchatMessage;

struct TransportWithDelay<D: Distribution<f32> + Send + Sync + 'static> {
    delay_secs: D,
    transport: Box<dyn Transport>,
}

pub trait DelayMillisDist: Distribution<f32> + Send + Sync + Clone + 'static {}

#[async_trait]
impl<D: DelayMillisDist> Transport for TransportWithDelay<D> {
    async fn open(&self, listen_addr: SocketAddr) -> Result<Box<dyn Socket>, TransportError> {
        let rng = SmallRng::from_rng(thread_rng()).unwrap();
        let socket = self.transport.open(listen_addr).await?;
        Ok(Box::new(SocketWithDelay {
            delay_secs: self.delay_secs.clone(),
            socket: Arc::new(RwLock::new(socket)),
            rng,
        }))
    }
}

struct SocketWithDelay<D: Distribution<f32> + Send + Sync + 'static> {
    delay_secs: D,
    socket: Arc<RwLock<Box<dyn Socket>>>,
    rng: SmallRng,
}

#[async_trait]
impl<D: DelayMillisDist> Socket for SocketWithDelay<D> {
    async fn send(
        &mut self,
        to: SocketAddr,
        message: ChitchatMessage,
    ) -> Result<(), TransportError> {
        let socket_clone = self.socket.clone();
        let delay_secs = self.delay_secs.sample(&mut self.rng);
        let delay = Duration::from_secs_f32(delay_secs);
        tokio::task::spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = socket_clone.write().await.send(to, message).await;
        });
        Ok(())
    }

    async fn recv(&mut self) -> Result<(SocketAddr, ChitchatMessage), TransportError> {
        self.socket.write().await.recv().await
    }
}

pub trait TransportExt {
    fn drop_message(self, drop_probability: f64) -> Box<dyn Transport>;
    fn delay<D: DelayMillisDist>(self, delay_proba: D) -> Box<dyn Transport>;
}

impl<T: Transport> TransportExt for T {
    fn drop_message(self, drop_probability: f64) -> Box<dyn Transport> {
        Box::new(TransportWithMessageDrop {
            drop_probability: Bernoulli::new(drop_probability).unwrap(),
            transport: Box::new(self),
        })
    }

    fn delay<D: DelayMillisDist>(self, delay_secs: D) -> Box<dyn Transport> {
        Box::new(TransportWithDelay {
            delay_secs,
            transport: Box::new(self),
        })
    }
}

struct TransportWithMessageDrop {
    drop_probability: Bernoulli,
    transport: Box<dyn Transport>,
}

#[async_trait]
impl Transport for TransportWithMessageDrop {
    async fn open(&self, listen_addr: SocketAddr) -> Result<Box<dyn Socket>, TransportError> {
        let rng = SmallRng::from_rng(thread_rng()).unwrap();
        let socket = self.transport.open(listen_addr).await?;
        Ok(Box::new(SocketWithMessageDrop {
            drop_probability: self.drop_probability,
            socket,
            rng,
        }))
    }
}

struct SocketWithMessageDrop {
    drop_probability: Bernoulli,
    socket: Box<dyn Socket>,
    rng: SmallRng,
}

#[async_trait]
impl Socket for SocketWithMessageDrop {
    async fn send(
        &mut self,
        to: SocketAddr,
        message: ChitchatMessage,
    ) -> Result<(), TransportError> {
        let should_drop = self.drop_probability.sample(&mut self.rng);
        if should_drop {
            return Ok(());
        }
        self.socket.send(to, message).await
    }

    async fn recv(&mut self) -> Result<(SocketAddr, ChitchatMessage), TransportError> {
        self.socket.recv().await
    }
}
