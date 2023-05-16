use age::x25519::{Identity, Recipient};
use bisky::lexicon::{
    app::bsky::feed::Post,
    com::atproto::repo::{Record, StrongRef},
};
use chrono::{TimeZone, Utc};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    backend::Backend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Span, Spans, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame, Terminal,
};
use std::{
    collections::{BTreeMap, HashMap},
    str::FromStr,
};
use tokio::sync::mpsc::{error::TryRecvError, Receiver};
use unicode_width::UnicodeWidthStr;
use uuid::Uuid;

use crate::{
    conf::Settings,
    errors::BmailError,
    key::{get_recipient_for_bskyer, decode, encode},
    message::{
        insert_with_collisions, BmailEnabledProfile, BmailLike, Conversation, DecryptedMessage,
        FirehoseMessages,
    },
    SharableBluesky,
};

pub enum InputMode {
    Normal,
    Editing,
    EditingRecipient,
    ScrollingMessages,
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
    /// Bluesky object for API Calls
    pub bluesky: SharableBluesky,
    /// Identity for Decrypting DMs
    pub identity: Identity,
    /// The currently active Conversation Id
    pub current_conversation_id: Option<Uuid>,
    /// Storage Medium for Conversations.
    pub conversations: HashMap<Uuid, Conversation>,
    /// Current state of the conversation
    pub conversation_state: ListState,
    /// Maps recipient DIDs to a specific Conversation UUID
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
        //TODO Get Conversation IDs and Recipient Lists from Profile

        Ok(())
    }
    /// Load a conversation. If there is a conversation with the recipients in memory, display messages. If there isn't one,
    /// check the profile self storage for a conversation that matches. If that fails, create a new conversation and upload it
    /// to the profile storage. This takes handles from the UI, so they'll be parsed into DIDs
    pub async fn load_conversation(
        &mut self,
        mut recipients: Vec<String>,
    ) -> Result<Uuid, BmailError> {
        // 0. Add myself to the participants
        recipients.push(self.conf.user.handle.clone());
        //println!("RECIPIENTS: {:?}", recipients);

        // 1. Get DIDs for recipients
        let participant_dids = {
            let mut dids: Vec<String> = Vec::with_capacity(recipients.len());
            let mut bsky = self.bluesky.0.write().await;
            let mut user = bsky.user(&self.conf.user.handle)?;
            for recipient in &recipients {
                let recipient_did = user.resolve_handle(recipient).await?;
                dids.push(recipient_did);
            }
            dids.sort();
            dids
        };
        //println!("DIDS: {:?}", participant_dids);

        // 2. Check if all DIDs are present in Conversation Storage as a key
        let mut conversation_id = None;
        for conversation in self.recipients_conversation_map.iter() {
            // We need the Vec as the HashMap Key to be sorted
            match conversation.0 == &participant_dids
            {
                true => {
                    conversation_id = Some(*conversation.1);
                    break;
                }
                false => continue,
            };
        }
        // 3. If conversation_id exists, set currently active conversation ID.
        if let Some(active_conversation_id) = conversation_id {
            self.current_conversation_id = Some(active_conversation_id);
        }
        // Else if, check if present in profile storage.
        else if let Ok(Some(c_id)) = self
            .get_cid_from_rc_map_from_profile(participant_dids.clone())
            .await
        {
            //println!("MY PROFILE");
            self.current_conversation_id = Some(c_id);

            self.recipients_conversation_map
            .insert(participant_dids.clone(), c_id);
        // Create a new conversation
        let conversation = Conversation {
            conversation_id: c_id,
            messages: BTreeMap::default(),
            recipient_active_time: HashMap::default(),
            participants: participant_dids.clone(),
        };
        self.conversations
            .insert(c_id, conversation);

        self.upload_rc_map_to_profile(participant_dids.clone(), c_id)
            .await?;

        // Check in participant profiles if a conversation exists. If it does, add it to our local storage
        } else if let Ok(Some(c_id)) = self.get_cid_from_rc_map_from_participants_profiles(participant_dids.clone()).await{
            //println!("THEIR PROFILE: {c_id}");
            self.current_conversation_id = Some(c_id);

            self.recipients_conversation_map
                .insert(participant_dids.clone(), c_id);
            // Create a new conversation
            let new_conversation = Conversation {
                conversation_id: c_id,
                messages: BTreeMap::default(),
                recipient_active_time: HashMap::default(),
                participants: participant_dids.clone(),
            };
            self.conversations
                .insert(c_id, new_conversation);

            self.upload_rc_map_to_profile(participant_dids.clone(), c_id)
                .await?;

        }
        // Else create a new one
        else {
            //println!("NEW");
            let new_conversation_id = Uuid::new_v4();

            self.current_conversation_id = Some(new_conversation_id);
            self.recipients_conversation_map
                .insert(participant_dids.clone(), new_conversation_id);
            // Create a new conversation
            let new_conversation = Conversation {
                conversation_id: new_conversation_id,
                messages: BTreeMap::default(),
                recipient_active_time: HashMap::default(),
                participants: participant_dids.clone(),
            };
            self.conversations
                .insert(new_conversation_id, new_conversation);

            self.upload_rc_map_to_profile(participant_dids.clone(), new_conversation_id)
                .await?;
        }

        // 4. Get all Conversation Records with that Conversation ID from each participant
        // 4.1 Check latest on server vs latest in memory(last entry)
        // 4.2 If server has newer messages, add missing to memory conversation
        if let Some(c_id) = self.current_conversation_id {
            // Create the conversation entry to update if it does not exist
            self.conversations
                .entry(c_id)
                .or_insert_with(|| Conversation {
                    conversation_id: c_id,
                    participants: participant_dids.clone(),
                    ..Default::default()
                });
            // This should always be true, because we create it above
            if let Some(conversation) = self.conversations.get_mut(&c_id) {
                conversation
                    .update_with_messages_from_participants(
                        self.bluesky.clone(),
                        &self.conf.user.handle,
                        &self.identity,
                        participant_dids,
                    )
                    .await?;

                // Set Conversation state of conversation
                self.conversation_state = ListState::default();
            }
        }

        Ok(self.current_conversation_id.unwrap())
    }

    /// We're storing a HashMap of recipients in a Conversation to conversation IDs in the profile(encrypted)
    /// so that multiple clients can fetch them, and so that we can recover them when the app is restarted.
    /// Get that Hashmap
    pub async fn get_rc_map_from_profile(
        &mut self,
        handle: &str,
    ) -> Result<Option<HashMap<Vec<String>, Uuid>>, BmailError> {
        let mut bsky = self.bluesky.0.write().await;
        let mut user = bsky.user(&self.conf.user.handle)?;
        let profile_record = user
            .get_record::<BmailEnabledProfile>(
                handle,
                "app.bsky.actor.profile",
                "self",
            )
            .await?;

        if let Some(r_map) = profile_record.value.bmail_rc_map {
             let decoded = decode(&r_map).await?;
            Ok(Some(decoded))
        } else {
            Ok(None)
        }
    }

    /// We're storing a HashMap of recipients in a Conversation to conversation IDs in the profile(encrypted)
    /// so that multiple clients can fetch them, and so that we can recover them when the app is restarted.
    /// Get that Hashmap
    pub async fn get_cid_from_rc_map_from_profile(
        &mut self,
        participants: Vec<String>,
    ) -> Result<Option<Uuid>, BmailError> {
        let handle = self.conf.user.handle.clone();
        let profile_rc_map = self.get_rc_map_from_profile(&handle).await?;
        if let Some(rc_map) = &profile_rc_map {
            let mut conversation_id = None;
            for key in rc_map.keys() {
                match key == &participants{
                // match key.iter().all(|item| participants.contains(item)) && participants.len() == keys.len() {
                    true => {
                        let c_id = rc_map.get(key).unwrap();
                        //println!("K P: {:?}||{:?}||{:?}", key, participants, c_id);
                        conversation_id = Some(*c_id);
                        break;
                    }
                    false => continue,
                };
            }
            Ok(conversation_id)
        } else {
            Ok(None)
        }
    }

    /// Check for a conversation ID in each participants profiles, so that if they make a conversation with me, and I try to make one later,
    /// it will find their ID and use it for my local conversation
    pub async fn get_cid_from_rc_map_from_participants_profiles(
        &mut self,
        participants: Vec<String>,
    ) -> Result<Option<Uuid>, BmailError> {


        for participant in participants.iter(){
            if let Some(user_did) = &self.user_did{
                // Skip if it's me
                if participant == user_did{
                    continue
                }
            }
            
            let profile_rc_map = self.get_rc_map_from_profile(participant).await?;
            //println!("profile_rc_map: {:?}", profile_rc_map);
            if let Some(rc_map) = &profile_rc_map {
                for key in rc_map.keys() {
                    // Need to check length because we might have one vector contain all of another
                    match key == &participants {
                        true => {
                            let c_id = rc_map.get(key).unwrap();
                            //println!("FOUND KEYS: {:#?}", keys);
                            return Ok(Some(*c_id));
                        }
                        false => continue,
                    };
                }
            }
    }
    Ok(None)
}

    /// Get the current rc_map from profile and add a new value to it
    pub async fn upload_rc_map_to_profile(
        &mut self,
        key: Vec<String>,
        value: Uuid,
    ) -> Result<(), BmailError> {
        let handle = self.conf.user.handle.clone();

        let mut profile_record = {
            let mut bsky = self.bluesky.0.write().await;
            let mut user = bsky.user(&handle)?;
            user.get_record::<BmailEnabledProfile>(&handle, "app.bsky.actor.profile", "self").await?
        };

        match &mut profile_record.value.bmail_rc_map {
            Some(m) => {      
                let mut rc_map: HashMap<Vec<String>, Uuid> = decode(m).await?; 
                rc_map.insert(key, value);
                let encoded = encode(rc_map).await?;
                profile_record.value.bmail_rc_map = Some(encoded);     
            }
            None => {
                let mut new_rc_map = HashMap::new();
                new_rc_map.insert(key, value);
                let encoded_map = encode(new_rc_map).await?;
                profile_record.value.bmail_rc_map = Some(encoded_map);
            }
        };

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

   /// Get the current rc_map from profile and add a new value to it
   pub async fn delete_rc_map_from_profile(
    &mut self,
) -> Result<(), BmailError> {
    let handle = self.conf.user.handle.clone();

    let mut profile_record = {
        let mut bsky = self.bluesky.0.write().await;
        let mut user = bsky.user(&handle)?;
        user.get_record::<BmailEnabledProfile>(&handle, "app.bsky.actor.profile", "self").await?
    };


    profile_record.value.bmail_rc_map = None;     
    

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
    pub fn add_bmail_to_conversation(
        &mut self,
        conv_id: Uuid,
        msg: &DecryptedMessage,
    ) -> Result<(), BmailError> {
        match self.conversations.get_mut(&conv_id) {
            Some(c) => {
                insert_with_collisions(&mut c.messages, msg);
                // Set current state to newest message
                self.conversation_state.select(Some(c.messages.keys().count()-1));
                Ok(())
            }
            None => Err(BmailError::ConversationNotFound),
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

        // 0. Get DIDS for participants
        let participant_dids = {
            let mut dids: Vec<String> = Vec::with_capacity(recipients.len());
            let mut bsky = self.bluesky.0.write().await;
            let mut user = bsky.user(&self.conf.user.handle)?;
            for recipient in &recipients {
                let recipient_did = user.resolve_handle(recipient).await?;
                dids.push(recipient_did);
            }
            dids.sort();
            dids
        };
        // Create Message
        let msg = DecryptedMessage {
            created_at: Utc::now(),
            creator: user_did.clone(),
            message: msg.to_string(),
            conversation_id,
            recipients: participant_dids.clone(),
            version: 0,
            creator_handle: self.conf.user.handle.clone(),
        };
        let record = msg.into_bmail_record(self.bluesky.clone()).await?;
        // Send Bmail by creating a profile post with the contents
        {
            let mut bsky = self.bluesky.0.write().await;
            let mut me = bsky.me().map_err::<BmailError, _>(Into::into)?;
            me.create_record("app.bsky.actor.profile", None, None, None, record)
                .await?;
        };
        // Add the decrypted message to the Conversation
        match self.add_bmail_to_conversation(conversation_id, &msg) {
            Ok(_) => (),
            Err(BmailError::ConversationNotFound) => {
                self.status = "Failed to find conversation".to_string()
            }
            Err(e) => self.status = format!("Unexpected_error: {}", e.to_string()),
        };
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
            bluesky: SharableBluesky::default(),
            identity: Identity::generate(),
            message_rx: None,
            status: "ALL GOOD".to_string(),
            conversations: HashMap::new(),
            conf: Settings::default(),
            user_did: None,
            current_conversation_id: None,
            recipients_conversation_map: HashMap::new(),
            conversation_state: ListState::default(),
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
        if let Some(rx) = &mut app.message_rx {
            match &rx.try_recv() {
                Ok(m) => match m {
                    FirehoseMessages::Bmail(m) => {
                        // println!("FOUND BMAIL");
                        // println!("\x07");
                        let msg = m.into_decrypted_message(&app.identity).await?;
                        app.add_bmail_to_conversation(msg.conversation_id, &msg)?;
                    }
                    FirehoseMessages::BmailLike(_l) => (),
                },
                Err(TryRecvError::Empty) => (),
                Err(_) => return Err(BmailError::FirehoseProcessCrashed),
            }
        }

        terminal.draw(|f| ui(f, &mut app))?;

        if let Event::Key(key) = event::read()? {
            match app.input_mode {
                InputMode::Normal => match key.code {
                    KeyCode::Char('e') => {
                        app.input_mode = InputMode::Editing;
                    }
                    KeyCode::Char('m') => {
                        app.input_mode = InputMode::ScrollingMessages;
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
                        let Some(c_id) = &app.current_conversation_id else{
                            app.status = "No conversation is active".to_string();
                            continue
                        };
                        let recipients_input = &app.recipient;
                        let recipients =
                            recipients_input.split(',').map(|s| s.to_string()).collect();
                        let input = app.input.clone();
                        match app.send_bmail(*c_id, recipients, &input).await {
                            Ok(_) => app.input= "".to_string(),
                            Err(BmailError::MissingRecipient(r)) => {
                                app.status = format!("Recipient {} is not using Bmail", r)
                            }
                            Err(e) => {
                                app.status = format!("Unexpected Error: {:#?}", e.to_string())
                            }
                        };
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
                        let recipients_input = &app.recipient;
                        let recipients =
                            recipients_input.split(',').map(|s| s.to_string()).collect();
                        match app.load_conversation(recipients).await {
                            Ok(u) => app.status = format!("Loaded Conversation: {}", u),
                            Err(e) => {
                                app.status =
                                    format!("Failed to load conversation: {:?}", e.to_string())
                            }
                        };
                    }
                    _ => {}
                },
                InputMode::ScrollingMessages if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Esc => {
                        app.input_mode = InputMode::Normal;
                    }
                    KeyCode::Up => {
                        if let Some(c_id) = app.current_conversation_id {
                            let i = match app.conversation_state.selected() {
                                Some(i) => {
                                    if i == 0 {
                                        app.conversations.get(&c_id).unwrap().messages.len() - 1
                                    } else {
                                        i - 1
                                    }
                                }
                                None => 0,
                            };
                            app.conversation_state.select(Some(i));
                        }
                    }
                    KeyCode::Down => {
                        if let Some(c_id) = app.current_conversation_id {
                            let i = match app.conversation_state.selected() {
                                Some(i) => {
                                    if i >= app.conversations.get(&c_id).unwrap().messages.len() - 1
                                    {
                                        0
                                    } else {
                                        i + 1
                                    }
                                }
                                None => 0,
                            };
                            app.conversation_state.select(Some(i));
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
}

fn ui<B: Backend>(f: &mut Frame<B>, app: &mut App) {
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
                Span::raw(" to start typing a message, "),
                Span::styled("m", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(" to enter conversation scroll mode."),
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
                Span::raw(" to load the conversation"),
            ],
            Style::default(),
        ),
        InputMode::ScrollingMessages => (
            vec![
                Span::raw("Press "),
                Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(" to stop Scrolling, "),
                Span::styled(
                    "Up/Dwn Arrow",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(" to scroll messages, "),
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
            InputMode::ScrollingMessages => Style::default(),
        })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Recipient Handle"),
        );
    f.render_widget(recipient, chunks[2]);

    let status = Paragraph::new(app.status.as_ref()).style(Style::default());
    f.render_widget(status, chunks[3]);

    let messages: Vec<ListItem> = match app.current_conversation_id {
        Some(c_id) => match app.conversations.get(&c_id) {
            Some(c) => c
                .messages
                .iter()
                .map(|(k, v)| {
                    let content = Spans::from(Span::raw(format!(
                        "{} {}: {}",
                        k.created_at.format("%Y/%m/%d %H:%M"), v.creator_handle, v.message
                    )));
                    ListItem::new(content)
                })
                .collect(),
            None => Default::default(),
        },
        None => Vec::new(),
    };

    let messages = List::new(messages)
        .style(match app.input_mode {
            InputMode::Normal => Style::default(),
            InputMode::Editing => Style::default(),
            InputMode::EditingRecipient => Style::default(),
            InputMode::ScrollingMessages => Style::default().fg(Color::Yellow),
        })
        .block(Block::default().borders(Borders::ALL).title("Messages"));
    f.render_stateful_widget(messages, chunks[4], &mut app.conversation_state);

    let input = Paragraph::new(app.input.as_ref())
        .style(match app.input_mode {
            InputMode::Normal => Style::default(),
            InputMode::Editing => Style::default().fg(Color::Yellow),
            InputMode::EditingRecipient => Style::default(),
            InputMode::ScrollingMessages => Style::default(),
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
        InputMode::ScrollingMessages => {}
    }
}
