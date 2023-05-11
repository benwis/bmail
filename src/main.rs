use age::x25519::Identity;
use bisky::atproto::{ClientBuilder, UserSession};
use bisky::firehose::cbor::Body as FirehoseBody;
use bisky::storage::File;
use bmail::conf::{get_configuration, Settings};
use bmail::errors::BmailError;
use bmail::key::get_identity;
use bmail::message::{BmailMessageRecord, DecryptedMessage};
use bmail::ui::{run_app, App};
use bmail::SharableBluesky;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt as _;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::mpsc::{self, Sender};
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use url::Url;

#[tokio::main]
async fn main() -> Result<(), BmailError> {
    let (socket, _response) = tokio_tungstenite::connect_async(
        Url::parse("wss://bsky.social/xrpc/com.atproto.sync.subscribeRepos").unwrap(),
    )
    .await
    .unwrap();

    // Create a new channel to send Posts from the Firehose thread
    let (tx, rx) = mpsc::channel(32);

    let conf = get_configuration()?;
    let identity = get_identity(&conf.key.file_path)?;
    println!("Pubkey: {}", identity.to_public());

    let storage = Arc::new(File::<UserSession>::new(PathBuf::from(
        "keys/bsky_creds.secret",
    )));
    let mut client = ClientBuilder::default()
        .session(None)
        .storage(storage)
        .build()
        .unwrap();

    client
        .login(
            &Url::parse("https://bsky.social").unwrap(),
            &conf.user.handle,
            &conf.user.password,
        )
        .await
        .unwrap();

    let bsky = SharableBluesky::new(client);
    let user_did = {
        let mut bsky_client = bsky.0.write().await;
        bsky_client
            .user(&conf.user.handle)?
            .resolve_handle(&conf.user.handle)
            .await?
    };

    // create app and run it
    let mut app = App {
        bluesky: bsky.clone(),
        identity: identity.clone(),
        message_rx: Some(rx),
        user_did: Some(user_did),
        conf: conf.clone(),
        ..Default::default()
    };

    // Initialize Profile for Bmail Message Sending
    app.initialize().await?;
    println!("HI");

    // A new task is spawned for processing firehose messages. The socket is
    // moved to the new task and processed there.
    let _firehose = tokio::spawn(async move {
        let _ = process_message(socket, tx, bsky.clone(), identity.clone(), conf).await;
    });

    // firehose.await.unwrap();

    // setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_app(&mut terminal, app).await;

    // restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{:?}", err)
    }

    Ok(())
}

pub async fn process_message(
    mut socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    tx: Sender<DecryptedMessage>,
    bsky: SharableBluesky,
    identity: Identity,
    conf: Settings,
) -> Result<(), BmailError> {
    while let Some(Ok(Message::Binary(message))) = socket.next().await {
        let mut bsky = bsky.0.write().await;
        let (_header, body) = bisky::firehose::cbor::read(&message).unwrap();
        if let FirehoseBody::Commit(commit) = body {
            if commit.operations.is_empty() {
                continue;
            }
            let operation = &commit.operations[0];
            if !operation.path.starts_with("app.bsky.feed.post/") {
                continue;
            }
            if let Some(cid) = operation.cid {
                let mut car_reader = Cursor::new(commit.blocks);
                let _car_header = bisky::firehose::car::read_header(&mut car_reader).unwrap();
                let car_blocks = bisky::firehose::car::read_blocks(&mut car_reader).unwrap();

                let record_reader = Cursor::new(car_blocks.get(&cid).unwrap());
                let bmail = serde_cbor::from_reader::<serde_cbor::Value, _>(record_reader)
                    .map_err::<BmailError, _>(Into::into)?;
                // println!("{bmail:?}");
                if bmail.bmail_type.is_some() {
                    println!("\n\nFOUND A BMAIL!\n\n");
                    if let Some(cipher_text) = bmail.bmail_cipher_text {
                        tx.send(message_struct)
                            .await
                            .map_err::<BmailError, _>(Into::into)?;
                    }
                }
            }
        }
    }
    Ok(())
}
