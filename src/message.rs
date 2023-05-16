use crate::{
    errors::BmailError,
    key::{decrypt_and_decode, encrypt_and_encode, get_recipient_for_bskyer},
    SharableBluesky,
};
use age::{x25519::Identity, Recipient as RecipientTrait};
use bisky::lexicon::com::atproto::repo::{Blob, StrongRef};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::from_value;
use std::{collections::BTreeMap, collections::HashMap};
use uuid::Uuid;

#[derive(Default)]
pub struct Conversation {
    /// A Unique ID for the Conversation this is a part of, to make it easier for clients to poll a particular conversation. Multiple Records might have the same ID, this means they are participants of the same chain
    pub conversation_id: Uuid,
    pub messages: BTreeMap<MessageKey, DecryptedMessage>,
    /// The currently active index for each recipient as seen by this client
    /// This way, we can filter out all the processed messages.
    /// Keyed by DID and contains an BTreeMap index
    pub recipient_active_time: HashMap<String, DateTime<Utc>>,
    /// The DID of the participants in a Conversation. Used so we know whose accounts to try to find messages on.
    pub participants: Vec<String>,
}

impl Conversation {
    /// Get Message Records for each Participant. If newer messages exist, add them to the local conversation
    /// This is run on initial conversation load in the UI, in case it's been updated since you last viewed it
    /// by others or on another client
    pub async fn update_with_messages_from_participants(
        &mut self,
        bsky: SharableBluesky,
        user_handle: &str,
        identity: &Identity,
        participant_dids: Vec<String>,
    ) -> Result<(), BmailError> {
        let mixer_map: BTreeMap<MessageKey, DecryptedMessage> = BTreeMap::new();
        // 0. Get date of latest message for each participant from storage
        // This is covered by the recipient_active_time field
        // 1. Get All Message Records for each participant
        let mut bsky = bsky.0.write().await;
        let mut user = bsky.user(user_handle)?;

        // Is this cursed? Probably. Am I going to fix it now? Obviously not
        for participant in participant_dids.iter() {
            let records = user
                .list_all_records::<serde_json::Value>("app.bsky.actor.profile", participant, true)
                .await?;

            // Parse into final value
            let mut bmail_records: Vec<BmailMessageRecord> = records
                .into_iter()
                .filter_map(|record| {
                    if let serde_json::Value::Object(r) = &record.value {
                        if r.get("bmail_type")
                            == Some(&serde_json::Value::String("bmail".to_string()))
                        {
                            from_value(record.value).ok()
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .collect();

            // 1.1. Filter by Conversation ID
            bmail_records.drain_filter(|r| r.bmail_conversation_id != self.conversation_id);
            // 1.2. Drop/Drain any that are older than the latest for each participant
            // TODO: This is being skipped because active_time is not updated
            let latest_post = self.recipient_active_time.get(&participant.to_string());
            if let Some(latest_post) = latest_post {
                bmail_records.drain_filter(|r| &r.bmail_created_at <= latest_post);
            }
            // 1.3 Add Them to the Mixer Map
            for record in bmail_records.into_iter() {
                let d_msg = record.into_decrypted_message(identity).await?;
                insert_with_collisions(&mut self.messages, &d_msg);
            }
        }

        //2. Drain mixer_map into conversation
        mixer_map
            .into_iter()
            .for_each(|(_k, v)| insert_with_collisions(&mut self.messages, &v));
        
        Ok(())
    }
}

/// Keeps track of the messages seen on this client
pub struct MyConversationPortion {
    pub conversation_id: Uuid,
    pub messages: BTreeMap<DateTime<Utc>, DecryptedMessage>,
}

//Data structure for a Bmail Message key
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct MessageKey {
    pub created_at: DateTime<Utc>,
    pub count: u32,
}
impl MessageKey {
    pub fn new() -> Self {
        Self {
            created_at: Utc::now(),
            count: 0,
        }
    }
    pub fn new_with_count(count: u32, created_at: &DateTime<Utc>) -> Self {
        Self {
            created_at: *created_at,
            count,
        }
    }
    pub fn update_count(&mut self, count: u32) -> &Self {
        self.count = count;
        self
    }
}

/// Record type that can encoded for a Bmail Message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BmailMessageRecord {
    // The type of the Record, might be used by something later
    #[serde(rename(serialize = "$type", deserialize = "$type"))]
    pub rust_type: String,
    pub bmail_created_at: DateTime<Utc>,
    pub bmail_conversation_id: Uuid,
    pub bmail_cipher_text: String,
    pub bmail_type: String,
    pub bmail_creator: String,
    pub bmail_creator_handle: String,
    pub bmail_version: usize,
    pub bmail_recipients: Vec<String>,
}

impl BmailMessageRecord {
    /// Convert a Bmail message from a BmailMessageRecord into a Message struct
    pub async fn into_decrypted_message(
        &self,
        identity: &Identity,
    ) -> Result<DecryptedMessage, BmailError> {
        let binary_message = decrypt_and_decode(identity, &self.bmail_cipher_text).await?;

        Ok(DecryptedMessage {
            created_at: self.bmail_created_at,
            creator: self.bmail_creator.clone(),
            creator_handle: self.bmail_creator_handle.clone(),
            conversation_id: self.bmail_conversation_id,
            message: String::from_utf8(binary_message).map_err(BmailError::FromStringError)?,
            recipients: self.bmail_recipients.clone(),
            version: self.bmail_version,
        })
    }
}

/// Record type that can encoded for a Bmail Message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirehoseBmailMessageRecord {
    /// The type of the Record for Bluesky. We don't really use it, but we might later
    #[serde(rename(serialize = "$type", deserialize = "$type"))]
    pub rust_type: String,
    pub bmail_created_at: DateTime<Utc>,
    pub bmail_conversation_id: String,
    pub bmail_cipher_text: String,
    pub bmail_type: String,
    pub bmail_creator: String,
    pub bmail_creator_handle: String,
    pub bmail_version: usize,
    pub bmail_recipients: Vec<String>,
}

impl TryFrom<FirehoseBmailMessageRecord> for BmailMessageRecord {
    type Error = BmailError;
    fn try_from(message: FirehoseBmailMessageRecord) -> Result<Self, Self::Error> {
        Ok(Self {
            bmail_conversation_id: Uuid::parse_str(&message.bmail_conversation_id)?,
            rust_type: message.rust_type,
            bmail_created_at: message.bmail_created_at,
            bmail_cipher_text: message.bmail_cipher_text,
            bmail_type: message.bmail_type,
            bmail_creator: message.bmail_creator,
            bmail_creator_handle: message.bmail_creator_handle,
            bmail_version: message.bmail_version,
            bmail_recipients: message.bmail_recipients,
        })
    }
}
//Data structure for a single Bmail
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecryptedMessage {
    pub created_at: DateTime<Utc>,
    pub creator: String,
    pub creator_handle: String,
    pub conversation_id: Uuid,
    pub message: String,
    pub recipients: Vec<String>,
    pub version: usize,
}

impl DecryptedMessage {
    /// Convert a Decrypted Message(PlainText) into a BmailProfileRecord(Encrypted)
    pub async fn into_bmail_record(
        &self,
        bsky: SharableBluesky,
    ) -> Result<BmailMessageRecord, BmailError> {
        let recipient_keys = {
            let mut keys: Vec<Box<dyn RecipientTrait + Send>> =
                Vec::with_capacity(self.recipients.len());
            for recipient in &self.recipients {
                let (recipient_key, _profile_record) =
                    get_recipient_for_bskyer(bsky.clone(), recipient).await?;
                if let Some(key) = recipient_key {
                    keys.push(Box::new(key));
                } else {
                    return Err(BmailError::MissingRecipient(recipient.to_string()));
                }
            }
            keys
        };

        let encoded = encrypt_and_encode(recipient_keys, self.message.as_bytes()).await?;

        Ok(BmailMessageRecord {
            bmail_created_at: self.created_at,
            bmail_conversation_id: self.conversation_id,
            bmail_cipher_text: encoded,
            bmail_type: "bmail".to_string(),
            bmail_version: self.version,
            bmail_recipients: self.recipients.clone(),
            bmail_creator: self.creator.clone(),
            rust_type: "app.bsky.actor.profile".to_string(),
            bmail_creator_handle: self.creator_handle.clone(),
        })
    }
}

/// Record type that can be passed to create_record() to store identity info(public_key) about the sender in a user profile.
/// Probably wrapped in Record<BmailEnbaledProfile>
#[derive(Debug, Serialize, Deserialize)]
pub struct BmailEnabledProfile {
    #[serde(rename(deserialize = "$type", serialize = "$type"))]
    pub rust_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar: Option<Blob>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub banner: Option<Blob>,
    pub description: Option<String>,
    #[serde(rename(deserialize = "displayName", serialize = "displayName"))]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bmail_pub_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bmail_notification_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bmail_notification_cid: Option<String>,
    pub bmail_rc_map: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BmailLike {
    #[serde(rename(deserialize = "createdAt"))]
    #[serde(rename(serialize = "createdAt"))]
    pub created_at: DateTime<Utc>,
    pub subject: StrongRef,
    pub bmail_recipients: Vec<String>,
    pub bmail_conversation_id: Uuid,
    pub bmail_type: String,
}

/// Function that can be recursed over to insert into a BTreeMap with possible collisions
/// If the value is present at the key, skip insert
pub fn insert_with_collisions(
    map: &mut BTreeMap<MessageKey, DecryptedMessage>,
    msg: &DecryptedMessage,
) {
    let mut count = 0;
    let mut key = MessageKey::new_with_count(count, &msg.created_at);
    loop {
        // If the key is present, and the messages match, we don't need to insert it again
        if map.contains_key(&key){
            let val = map.get(&key).unwrap(); 
            if val == msg{
                return;
            } 
        }
        // If the key is not present, then we need to insert it 
        else if !map.contains_key(&key) {
            map.insert(key.clone(), msg.clone());
            return;
        }
        count += 1;
        key.update_count(count);
    }
}

/// The types of things the Firehose might send and be returned from the Firehose thread
#[derive(Debug, Clone)]
pub enum FirehoseMessages {
    Bmail(BmailMessageRecord),
    BmailLike(BmailLike),
}
