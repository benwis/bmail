use std::sync::{Arc};

use bisky::{bluesky::Bluesky, atproto::Client, lexicon::app::bsky::actor::ProfileViewDetailed};
use tokio::sync::{oneshot, RwLock};

pub mod key;
pub mod dm;
pub mod conf;
pub mod errors;
pub mod ui;
pub mod message;

pub struct SharableBluesky(pub Arc<RwLock<Bluesky>>);

impl SharableBluesky{
    pub fn new(client: Client) -> Self{
        Self(Arc::new(RwLock::new(Bluesky::new(client))))
    }
}

impl Clone for SharableBluesky{
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// Provided by the requester and used by the manager task to send
/// the command response back to the requester.
type Responder<T> = oneshot::Sender<T>;

#[derive(Debug)]
pub enum Command {
    GetProfile {
        key: String,
        resp: Responder<ProfileViewDetailed>,
    },
}