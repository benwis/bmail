use age::{x25519::{Identity, Recipient}, Recipient as RecipientTrait};
use bisky::lexicon::{app::bsky::feed::Post, com::atproto::repo::Record};
use chrono::Utc;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    backend::Backend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Span, Spans, Text},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame, Terminal,
};
use regex::Regex;
use std::{io::Write, str::FromStr};
use tokio::sync::mpsc::Receiver;
use unicode_width::UnicodeWidthStr;

use crate::{
    conf::Settings,
    errors::BmailError,
    message::{ConversationPortion, Message, BmailEnabledProfile},
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
    /// The currently active Conversation
    pub current_recipient: Option<Vec<Recipient>>,
    /// Channel for Receiving Messages
    pub message_rx: Option<Receiver<Message>>,
    /// Storage Medium for ConversationPortions 
    pub conversations: Vec<ConversationPortion>,
    /// App Settings
    pub conf: Settings,
    /// The DID of the current user
    pub user_did: Option<String>,
}

impl App {

    /// Initialize profile 
    pub async fn initialize(&mut self) -> Result<(), BmailError>{
        //Get Profile and check for existence of key
        let profile_record = {
            let mut bsky = self.bluesky.0.write().await;
            let mut user = bsky.user(&self.conf.user.handle)?;
            user.get_record::<BmailEnabledProfile>(&self.conf.user.handle, "app.bsky.actor.profile", "self").await?
        };
        if profile_record.value.bmail_pub_key.is_none(){
            self.upload_bmail_recipient().await?;
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

        let profile_record = user.get_record::<BmailEnabledProfile>(handle, "app.bsky.actor.profile", "self").await?;
        let recipient = match &profile_record.value.bmail_pub_key{
            Some(k) =>  Some(Recipient::from_str(k).map_err(|_| BmailError::ParseRecipientError)?),
            None => None
        };
        Ok((recipient, profile_record))
    }

    /// Create a BmailEnabledProfile Profile Record to store your Recipient. 
    pub async fn upload_bmail_recipient(&mut self) -> Result<(), BmailError>{
        let handle = &self.conf.user.handle.clone();

        //Get existing Record so we can only change one thing
        let (recipient, mut profile_record) = match self.get_recipient_for_bskyer(handle).await{
            Ok(r) => r,
            Err(r) => return Err(BmailError::InternalServerError),
        };

        //Update pub key
        profile_record.value.bmail_pub_key = Some(self.identity.to_public().to_string());

        let mut bsky = self.bluesky.0.write().await;
        let mut me = bsky.me()?;
        me.put_record("app.bsky.actor.profile", "self", None, None, Some(&profile_record.cid), &profile_record.value).await?;
        
        Ok(())

    }
    /// Send a Bmail by adding your message to your ConversationPortion in your profile Record
    pub async fn send_bmail(&mut self, recipients: Vec<&str>, msg: &str) -> Result<(), BmailError> {
        
        let recipient_keys = {
            let mut keys: Vec<Box<dyn RecipientTrait + Send>> = Vec::with_capacity(recipients.len());
            for recipient in recipients{
                let (recipient_key, _profile_record) = self.get_recipient_for_bskyer(recipient).await?; 
                if let Some(key) = recipient_key{
                    keys.push(Box::new(key));
                } else{
                    return Err(BmailError::MissingRecipient(recipient.to_string()))
                }
            };
            keys
        };

        let mut bsky = self.bluesky.0.write().await;
        let me = bsky.me().map_err::<BmailError, _>(Into::into)?;
        println!("Message Size: {}", msg.len());
        println!("Message Char Count: {}", msg.chars().count());

        // Encrypt the plaintext to a ciphertext...
        let encryptor = age::Encryptor::with_recipients(recipient_keys)
            .expect("we provided a recipient");

        let mut encrypted = vec![];
        let mut writer = encryptor
            .wrap_output(&mut encrypted)
            .map_err::<BmailError, _>(Into::into)?;
        writer.write_all(msg.as_bytes())?;
        writer.finish()?;
        println!("Number of Bytes: {}", encrypted.len());

        use base64::{engine::general_purpose, Engine as _};
        let encoded: String = general_purpose::STANDARD_NO_PAD.encode(&encrypted);
        // Construct Dm
        // //dm::@recipient::message
        // .len() is number of bytes NOT # of chars.
        // let dm: String = format!("//dm::@{}::{:?}", recipient, encoded);
        // println!("DM: {}", dm);
        println!("Length: {}", encoded.chars().count());

        // let encoded2 = base2048::encode(&encrypted);
        // let dm2 = format!("//dm::@{}::{:?}", recipient, encoded2);

        // println!("DM2048: {}", dm2);
        // println!("Length2048: {}", dm2.chars().count());
        let Some(user_did) = &self.user_did else {
            return Err(BmailError::InternalServerError)
        };
        // Create Message
        let msg = Message{
            created_at: Utc::now(),
            creator: user_did.clone(),
            raw_message: msg.to_string(),
            message: encoded,
        };
        // Add Message to ConversationPortion
        // Upload Conversation portion
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
            conversations: vec![ConversationPortion::default()],
            conf: Settings::default(),
            current_recipient: None,
            user_did: None,
        }
    }
}

pub async fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    mut app: App,
) -> Result<(), BmailError> {
    loop {
        //TODO: Read Messages from Message Channel and add to Conversations

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
                        let recipient_input = &app.recipient.clone();
                        let msg = &app.input.clone();

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
                Span::raw("to change conversation, "),
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
