use age::{x25519::{Identity, Recipient}};
use bisky::lexicon::app::bsky::feed::Post;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
};
use ratatui::{
    backend::Backend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Span, Spans, Text},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame, Terminal,
};
use regex::Regex;
use tokio::sync::mpsc::Receiver;
use std::{io::Write, str::FromStr};
use unicode_width::UnicodeWidthStr;

use crate::{SharableBluesky, message::{Message, Conversations}, errors::BmailError, conf::Settings};

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
    /// The currently active Recipient
    pub current_recipient: Option<Recipient>,
    /// Channel for Receiving Messages
    pub message_rx: Option<Receiver<Message>>,
    /// Storage Medium for Conversations and Messages
    pub conversations: Conversations,
    /// App Settings
    pub conf: Settings

}

impl App{
    /// Scrape the recipient's Profile for their Public Key so we can encrypt this thing
    pub async fn get_recipient_identity(&mut self, recipient: &str) -> Result<Recipient, BmailError>{
        let mut bsky = self.bluesky.0.write().await;
        let mut user = bsky.user(&self.conf.user.handle)?;
        let recipient_profile = user.get_profile_other(recipient).await?;
        let Some(description) = recipient_profile.description else {
            return Err(BmailError::MissingRecipientIdentity)
        };
        println!("Description: {}", &description);

        let re = Regex::new(r"bmail:(.*)\s").unwrap();
        let pubkey = match re.captures(&description){
            Some(r)=> r.get(0).unwrap().as_str(),
            None => return Err(BmailError::MissingRecipientIdentity)
        }.to_string();
        println!("Found Public Key: {}", &pubkey);
        let recipient = Recipient::from_str(&pubkey).map_err(|_| BmailError::ParseRecipientError)?;
        
        Ok(recipient)
    }
    /// Send a DM
    pub async fn send_dm(&mut self, recipient: &str, msg: &str) -> Result<(), BmailError>{
        let pubkey: Recipient = self.get_recipient_identity(recipient).await?;

        let mut bsky = self.bluesky.0.write().await;
        let mut me = bsky.me().map_err::<BmailError, _>(Into::into)?;


        // Encrypt the plaintext to a ciphertext...
        let encryptor = age::Encryptor::with_recipients(vec![Box::new(pubkey)])
            .expect("we provided a recipient");

        let mut encrypted = vec![];
        let mut writer = encryptor.wrap_output(&mut encrypted).map_err::<BmailError, _>(Into::into)?;
        writer.write_all(msg.as_bytes())?;
        writer.finish()?;

        // Construct Dm
        // //dm::@recipient::message
        let dm = format!("//dm::@{}::{:?}", recipient, encrypted);

        
        let post = Post{
            rust_type: Some("app.bsky.feed.post".to_string()),
            text: dm,
            created_at: None,
            embed: None,
            reply: None,
        };
        me.post(post).await?;
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
            conversations: Conversations::default(),
            conf: Settings::default(),
            current_recipient: None,
        }
    }
}

pub async fn run_app<B: Backend>(terminal: &mut Terminal<B>, mut app: App) -> Result<(), BmailError> {
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
                        //TODO: Create and Send Post
                        let recipient = &app.recipient.clone();
                        let msg = &app.input.clone();
                        app.send_dm(recipient, msg).await?;
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
                    },
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
    .block(Block::default().borders(Borders::ALL).title("Recipient Handle"));
    f.render_widget(recipient, chunks[2]);

    let status = Paragraph::new(app.status.as_ref())
    .style(Style::default());
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
    },
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