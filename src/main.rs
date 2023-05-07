use age::x25519::Identity;
use bisky::firehose::cbor::Body as FirehoseBody;
use bisky::firehose::models::FirehosePost;
use bisky::atproto::{ClientBuilder, UserSession};
use bisky::{storage::File};
use bmail::message::Message as BMessage;
use url::Url;
use bmail::conf::{get_configuration, Settings};
use bmail::errors::BmailError;
use bmail::ui::{App, run_app};
use bmail::key::get_identity;
use bmail::SharableBluesky;
use futures::{StreamExt as _};
use tokio::net::TcpStream;
use tokio_tungstenite::{WebSocketStream, MaybeTlsStream};
use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::sync::Arc;
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio::sync::mpsc::{self, Sender};
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{CrosstermBackend},
    Terminal,
};
use std::io;

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

    let storage = Arc::new(File::<UserSession>::new(PathBuf::from("keys/bsky_creds.secret")));
    let mut client= ClientBuilder::default().session(None).storage(storage).build().unwrap();

    client.login(&Url::parse("https://bsky.social").unwrap(), &conf.user.handle, &conf.user.password)
    .await
    .unwrap();

    let bsky = SharableBluesky::new(client);

    // create app and run it
    let app = App { bluesky: bsky.clone(), identity: identity.clone(), message_rx: Some(rx), ..Default::default() };
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

pub async fn process_message(mut socket: WebSocketStream<MaybeTlsStream<TcpStream>>, tx: Sender<BMessage>, bsky: SharableBluesky, identity: Identity, conf: Settings) -> Result<(), BmailError>{
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
                    let post = serde_cbor::from_reader::<FirehosePost, _>(record_reader).map_err::<BmailError, _>(Into::into).unwrap();
                    // println!("{post:?}");
                    if post.text.starts_with("/dm"){
                        println!("\n\nFOUND A DM LOOK AT ME\n\n");
                        println!("Text: {}",&post.text);
                        if post.text.contains(&format!("@{}",&conf.user.handle)){
                            let poster_did = commit.repo;
                            let mut user = bsky.user(&conf.user.handle).unwrap();
                            let sender = user.get_profile_other(&poster_did).await.expect("Failed to get profile for known user!");
                            let sender_handle = sender.handle;
                            let raw_text: Vec<&str> = post.text.split("::").collect();
                            
                            if raw_text.len() != 3 {
                             // Malformed DM
                             println!("MALFORMED DM");
                             continue;
                            }
                            let command = raw_text[0];
                            let recipient = raw_text[1];
                            let message = raw_text[2];
                            println!("{command}::{sender_handle}::{recipient}::{message}");
                            // Decrypt Message
                            let decrypted = {
                                let decryptor = match age::Decryptor::new(message.as_bytes()).map_err::<BmailError, _>(Into::into)? {
                                    age::Decryptor::Recipients(d) => d,
                                    _ => unreachable!(),
                                };
                            
                                let mut decrypted = vec![];
                                let mut reader = decryptor.decrypt(std::iter::once(&identity as &dyn age::Identity)).map_err::<BmailError, _>(Into::into)?;
                                reader.read_to_end(&mut decrypted).map_err::<BmailError, _>(Into::into)?;
                            
                                decrypted
                            };
                            

                            let message_struct = BMessage{
                                command: command.to_string(),
                                recipient_did: recipient.to_string(),
                                sender_did: sender_handle,
                                created_at: post.created_at,
                                raw_message: message.to_string(),
                                message: Some(String::from_utf8(decrypted).map_err::<BmailError, _>(Into::into)?),

                            };
                            tx.send(message_struct).await.map_err::<BmailError, _>(Into::into)?;
                            
                        }
                    }
                }
            }
    }
    Ok(())
}