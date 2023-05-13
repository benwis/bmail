use age::{
    x25519::{Identity, Recipient},
};
use bisky::lexicon::{
    app::bsky::feed:: Post,
    com::atproto::repo::{Record, StrongRef},
};
use chrono::{TimeZone, Utc};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    backend::Backend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Span, Spans, Text},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame, Terminal,
};
use std::{collections::HashMap, str::FromStr};
use tokio::sync::mpsc::{Receiver, error::TryRecvError};
use unicode_width::UnicodeWidthStr;
use uuid::Uuid;

use crate::{
    conf::Settings,
    errors::BmailError,
    key::get_recipient_for_bskyer,
    message::{BmailEnabledProfile, BmailLike, Conversation, DecryptedMessage, FirehoseMessages, insert_with_collisions},
    SharableBluesky,
};

pub enum InputMode {
    Normal,
    Editing,
    EditingRecipient,
}

/// App holds the state of the application
pub struct App {
    /// Current value of the recipient box
    pub recipient: String,
    /// Current value of the input box
    pub input: String,
    /// Current value of the status field
    pub status: String,
    /// Current input mode
    pub input_mode: InputMode,
    /// History of recorded messages
    pub messages: Vec<String>,
    /// Bluesky object for API Calls
    pub bluesky: SharableBluesky,
    /// Identity for Decrypting DMs
    pub identity: Identity,
    /// The currently active Conversation Id
    pub current_conversation_id: Option<Uuid>,
    /// Storage Medium for Conversations.
    pub conversations: HashMap<Uuid, Conversation>,
    pub recipients_conversation_map: HashMap<Vec<String>, Uuid>,
    /// Channel for Receiving Messages
    pub message_rx: Option<Receiver<FirehoseMessages>>,
    /// App Settings
    pub conf: Settings,
    /// The DID of the current user
    pub user_did: Option<String>,
}

impl App {
    /// Initialize profile
    pub async fn initialize(&mut self) -> Result<(), BmailError> {
        //Get Profile and check for existence of key
        let profile_record = {
            let mut bsky = self.bluesky.0.write().await;
            let mut user = bsky.user(&self.conf.user.handle)?;
            user.get_record::<BmailEnabledProfile>(
                &self.conf.user.handle,
                "app.bsky.actor.profile",
                "self",
            )
            .await?
        };
        
        if profile_record.value.bmail_pub_key.is_none() {
            self.upload_bmail_recipient().await?;
        }
        if profile_record.value.bmail_notification_uri.is_none() {
            self.create_notification_post().await?;
        }

        Ok(())
    }
    /// Load a conversation. If there is a conversation with the recipients in memory, display messages. If there isn't one,
    /// check the profile self storage for a conversation that matches. If that fails, create a new conversation and upload it 
    /// to the profile storage. This takes handles from the UI, so they'll be parsed into DIDs
    pub async fn load_conversation(&mut self, recipients: Vec<String>) -> Result<(), BmailError>{
        // 1. Get DIDs for recipients
        let recipient_dids = {
            let mut dids: Vec<String> =
                Vec::with_capacity(recipients.len());
            let mut bsky = self.bluesky.0.write().await;
            let mut user = bsky.user(&self.conf.user.handle)?;
            for recipient in &recipients {
                let recipient_did = user.resolve_handle(recipient).await?;
                dids.push(recipient_did);
            }
            dids
        };

        // 2. Check if all DIDs are present in Conversation Storage as a key
        let mut conversation_id = None;
        for conversation in self.recipients_conversation_map.iter(){
            match conversation.0.iter().all(|item| recipient_dids.contains(item)){
                true => {
                    conversation_id = Some(*conversation.1);
                    break
                },
                false => continue,
            };
        }
        // 3. If conversation_id exists, set currently active conversation ID. Else, create a new one
        if let Some(active_conversation_id) = conversation_id{
            self.current_conversation_id = Some(active_conversation_id);
        } else {
            let new_conversation_id = Uuid::new_v4();
            self.current_conversation_id = Some(new_conversation_id);
            self.recipients_conversation_map.insert(recipient_dids.clone(), new_conversation_id);
        }

        // 4. Get all Conversation Records with that Conversation ID from each participant
        // 4.1 Check latest on server vs latest in memory(last entry)
        // 4.2 If server has newer messages, add missing to memory conversation
        if let Some(c_id) = self.current_conversation_id{
        if let Some(conversation) = self.conversations.get_mut(&c_id){
            conversation.update_with_messages_from_participants(self.bluesky.clone(), &self.conf.user.handle, &self.identity, recipient_dids).await?;
        }
    }
        Ok(())
    }

    /// Scrape the recipient's Profile for their Public Key so we can encrypt this thing
    pub async fn get_recipient_for_bskyer(
        &mut self,
        handle: &str,
    ) -> Result<(Option<Recipient>, Record<BmailEnabledProfile>), BmailError> {
        let mut bsky = self.bluesky.0.write().await;
        let mut user = bsky.user(&self.conf.user.handle)?;

        let profile_record = user
            .get_record::<BmailEnabledProfile>(handle, "app.bsky.actor.profile", "self")
            .await?;
        let recipient = match &profile_record.value.bmail_pub_key {
            Some(k) => Some(Recipient::from_str(k).map_err(|_| BmailError::ParseRecipientError)?),
            None => None,
        };
        Ok((recipient, profile_record))
    }

    /// Create an extremely old post that can be liked to indicate you have a new Bmail
    pub async fn create_notification_post(&mut self) -> Result<(), BmailError> {
        let handle = &self.conf.user.handle.clone();

        let notif_post = {
            let mut bsky = self.bluesky.0.write().await;
            let mut me = bsky.me()?;
            me.create_record(
                "app.bsky.feed.post",
                None,
                None,
                None,
                Post {
                    rust_type: Some("app.bsky.feed.post".to_string()),
                    text: "You've got Bmail".to_string(),
                    created_at: Utc.with_ymd_and_hms(1970, 1, 1, 0, 0, 0).unwrap(),
                    embed: None,
                    reply: None,
                },
            )
            .await
            .unwrap()
        }; //Get existing Record so we can only change one thing

        let (_recipient, mut profile_record) = match self.get_recipient_for_bskyer(handle).await {
            Ok(r) => r,
            Err(_) => return Err(BmailError::InternalServerError),
        };

        profile_record.value.bmail_notification_uri = Some(notif_post.uri);
        profile_record.value.bmail_notification_cid = Some(notif_post.cid);

        let mut bsky = self.bluesky.0.write().await;
        bsky.me()?
            .put_record(
                "app.bsky.actor.profile",
                "self",
                None,
                None,
                Some(&profile_record.cid),
                &profile_record.value,
            )
            .await?;

        Ok(())
    }

    /// Create a BmailEnabledProfile Profile Record to store your Recipient.
    pub async fn upload_bmail_recipient(&mut self) -> Result<(), BmailError> {
        let handle = &self.conf.user.handle.clone();

        //Get existing Record so we can only change one thing
        let (_recipient, mut profile_record) = match self.get_recipient_for_bskyer(handle).await {
            Ok(r) => r,
            Err(_) => return Err(BmailError::InternalServerError),
        };

        //Update pub key
        profile_record.value.bmail_pub_key = Some(self.identity.to_public().to_string());

        let mut bsky = self.bluesky.0.write().await;
        let mut me = bsky.me()?;
        me.put_record(
            "app.bsky.actor.profile",
            "self",
            None,
            None,
            Some(&profile_record.cid),
            &profile_record.value,
        )
        .await?;

        Ok(())
    }

    /// Notify Recipients they have a Bmail by liking their bmail notification post
    pub async fn notify_recipients(
        &mut self,
        conversation_id: Uuid,
        recipients: Vec<String>,
    ) -> Result<(), BmailError> {
        for recipient in recipients.iter() {
            let (_recipient_key, profile_record) =
                get_recipient_for_bskyer(self.bluesky.clone(), recipient).await?;
            let mut bsky = self.bluesky.0.write().await;
            let mut me = bsky.me().map_err::<BmailError, _>(Into::into)?;
            me.create_record(
                "app.bsky.feed.like",
                None,
                None,
                None,
                BmailLike {
                    subject: StrongRef {
                        cid: profile_record
                            .value
                            .bmail_notification_cid
                            .ok_or_else(|| BmailError::MalformedBmail)?,
                        uri: profile_record
                            .value
                            .bmail_notification_uri
                            .ok_or_else(|| BmailError::MalformedBmail)?,
                    },
                    created_at: Utc::now(),
                    bmail_recipients: recipients.clone(),
                    bmail_conversation_id: conversation_id,
                    bmail_type: "notification".to_string(),
                },
            )
            .await?;
        }
        Ok(())
    }

    /// Add a single Bmail message to the Conversation
    pub fn add_bmail_to_conversation(&mut self, conv_id: Uuid, msg: &DecryptedMessage) -> Result<(), BmailError>{
        match self.conversations.get_mut(&conv_id){
            Some(c) => {
                insert_with_collisions(&mut c.messages, msg);
                Ok(())
            },
            None => Err(BmailError::ConversationNotFound)
        }
    }

    /// Send a Bmail by adding your message to your ConversationPortion in your profile Record
    pub async fn send_bmail(
        &mut self,
        conversation_id: Uuid,
        recipients: Vec<String>,
        msg: &str,
    ) -> Result<(), BmailError> {
        // println!("Message Size: {}", msg.len());
        // println!("Message Char Count: {}", msg.chars().count());

        let Some(user_did) = &self.user_did else {
            return Err(BmailError::InternalServerError)
        };
        // Create Message
        let msg = DecryptedMessage {
            created_at: Utc::now(),
            creator: user_did.clone(),
            message: msg.to_string(),
            conversation_id,
            recipients: recipients.clone(),
            version: 0,
        };
        let record = msg
            .into_bmail_record(self.bluesky.clone())
            .await?;
        // Send Bmail by creating a profile post with the contents
        {
            let mut bsky = self.bluesky.0.write().await;
            let mut me = bsky.me().map_err::<BmailError, _>(Into::into)?;
            me.create_record("app.bsky.actor.profile", None, None, None, record)
                .await?;
        };
        // Add the decrypted message to the Conversation
        self.add_bmail_to_conversation(conversation_id, &msg)?;
        // Notify recipients that we have sent them a Bmail
        self.notify_recipients(conversation_id, recipients).await?;

        Ok(())
    }
}

impl Default for App {
    fn default() -> App {
        App {
            input: String::new(),
            recipient: String::new(),
            input_mode: InputMode::Normal,
            messages: Vec::new(),
            bluesky: SharableBluesky::default(),
            identity: Identity::generate(),
            message_rx: None,
            status: "ALL GOOD".to_string(),
            conversations: HashMap::new(),
            conf: Settings::default(),
            user_did: None,
            current_conversation_id: None,
            recipients_conversation_map: HashMap::new(),
        }
    }
}

pub async fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    mut app: App,
) -> Result<(), BmailError> {
    loop {
        // Read messages on the channel and process them. Should work ok unless you're under
        // an absolute deluge. TODO: Be smarter about reading these messages
        if let Some(rx) = &mut app.message_rx{
            match &rx.try_recv(){
                Ok(m) => match m {
                    FirehoseMessages::Bmail(m) => {
                       let msg =  m.into_decrypted_message(&app.identity).await?;
                       app.add_bmail_to_conversation(msg.conversation_id, &msg)?;
                    },
                    FirehoseMessages::BmailLike(_l) => (),
                },
                Err(TryRecvError::Empty) => (),
                Err(_) => return Err(BmailError::FirehoseProcessCrashed),
            }
    }

        terminal.draw(|f| ui(f, &app))?;

        if let Event::Key(key) = event::read()? {
            match app.input_mode {
                InputMode::Normal => match key.code {
                    KeyCode::Char('e') => {
                        app.input_mode = InputMode::Editing;
                    }
                    KeyCode::Tab => {
                        app.input_mode = InputMode::EditingRecipient;
                    }
                    KeyCode::Char('q') => {
                        return Ok(());
                    }
                    _ => {}
                },
                InputMode::Editing if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Enter => {
                        //TODO: Create and Send Bmail
                        // let recipient_input = &app.recipient.clone();
                        // let msg = &app.input.clone();

                        // app.send_bmail(recipients, msg).await?;
                        app.messages.push(app.input.drain(..).collect());
                    }
                    KeyCode::Char(c) => {
                        app.input.push(c);
                    }
                    KeyCode::Backspace => {
                        app.input.pop();
                    }
                    KeyCode::Esc => {
                        app.input_mode = InputMode::Normal;
                    }
                    KeyCode::Tab => {
                        app.input_mode = InputMode::EditingRecipient;
                    }
                    _ => {}
                },
                InputMode::EditingRecipient if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Char(c) => {
                        app.recipient.push(c);
                    }
                    KeyCode::Backspace => {
                        app.recipient.pop();
                    }
                    KeyCode::Esc => {
                        app.input_mode = InputMode::Normal;
                    }
                    KeyCode::Tab => {
                        app.input_mode = InputMode::EditingRecipient;
                    }
                    KeyCode::Enter => {
                        //TODO: Get public key of new recipient and display success failure. Load old messages maybe.
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
}

fn ui<B: Backend>(f: &mut Frame<B>, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints(
            [
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(3),
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(3),
            ]
            .as_ref(),
        )
        .split(f.size());

    let title = Span::styled("Bmail", Style::default().add_modifier(Modifier::BOLD));
    let mut title_text = Text::from(title);
    title_text.patch_style(Style::default());
    let title_message = Paragraph::new(title_text);
    f.render_widget(title_message, chunks[0]);

    let (msg, style) = match app.input_mode {
        InputMode::Normal => (
            vec![
                Span::raw("Press "),
                Span::styled("q", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(" to exit, "),
                Span::styled("e", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(" to start typing message or choosing conversation."),
            ],
            Style::default().add_modifier(Modifier::RAPID_BLINK),
        ),
        InputMode::Editing => (
            vec![
                Span::raw("Press "),
                Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(" to stop Editing, "),
                Span::styled("Tab", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(" to change conversation, "),
                Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(" to send the message"),
            ],
            Style::default(),
        ),
        InputMode::EditingRecipient => (
            vec![
                Span::raw("Press "),
                Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(" to stop Editing, "),
                Span::styled("Tab", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(" to choose recipient, "),
                Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(" to send the message"),
            ],
            Style::default(),
        ),
    };
    let mut text = Text::from(Spans::from(msg));
    text.patch_style(style);
    let help_message = Paragraph::new(text);
    f.render_widget(help_message, chunks[1]);

    let recipient = Paragraph::new(app.recipient.as_ref())
        .style(match app.input_mode {
            InputMode::Normal => Style::default(),
            InputMode::Editing => Style::default(),
            InputMode::EditingRecipient => Style::default().fg(Color::Yellow),
        })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Recipient Handle"),
        );
    f.render_widget(recipient, chunks[2]);

    let status = Paragraph::new(app.status.as_ref()).style(Style::default());
    f.render_widget(status, chunks[3]);

    let messages: Vec<ListItem> = app
        .messages
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let content = Spans::from(Span::raw(format!("{}: {}", i, m)));
            ListItem::new(content)
        })
        .collect();

    let messages =
        List::new(messages).block(Block::default().borders(Borders::ALL).title("Messages"));
    f.render_widget(messages, chunks[4]);

    let input = Paragraph::new(app.input.as_ref())
        .style(match app.input_mode {
            InputMode::Normal => Style::default(),
            InputMode::Editing => Style::default().fg(Color::Yellow),
            InputMode::EditingRecipient => Style::default(),
        })
        .block(Block::default().borders(Borders::ALL).title("Input"));
    f.render_widget(input, chunks[5]);
    match app.input_mode {
        InputMode::Normal =>
            // Hide the cursor. `Frame` does this by default, so we don't need to do anything here
            {}

        InputMode::Editing => {
            // Make the cursor visible and ask ratatui to put it at the specified coordinates after rendering
            f.set_cursor(
                // Put cursor past the end of the input text
                chunks[5].x + app.input.width() as u16 + 1,
                // Move one line down, from the border to the input line
                chunks[5].y + 1,
            )
        }
        InputMode::EditingRecipient => {
            // Make the cursor visible and ask ratatui to put it at the specified coordinates after rendering
            f.set_cursor(
                // Put cursor past the end of the input text
                chunks[2].x + app.recipient.width() as u16 + 1,
                // Move one line down, from the border to the input line
                chunks[2].y + 1,
            )
        }
    }
}
