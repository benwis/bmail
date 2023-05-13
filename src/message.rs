use crate::{errors::BmailError, key::{get_recipient_for_bskyer, decrypt_and_decode, encrypt_and_encode}, SharableBluesky};
use age::{
    x25519::{Identity},
    Recipient as RecipientTrait,
};
use bisky::lexicon::com::atproto::repo::{Blob, StrongRef};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    collections::{HashMap},
};
use uuid::Uuid;
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
        pub async fn update_with_messages_from_participants(&mut self, bsky: SharableBluesky, user_handle: &str, identity: &Identity, participant_dids: Vec<String>) -> Result<(), BmailError> {
            let mixer_map: BTreeMap<MessageKey, DecryptedMessage> = BTreeMap::new();
            // 0. Get date of latest message for each participant from storage
            // This is covered by the recipient_active_time field
            // 1. Get All Message Records for each participant
            let mut bsky = bsky.0.write().await;
            let mut user = bsky.user(user_handle)?;
            
            // Is this cursed? Probably. Am I going to fix it now? Obviously not
            for participant in participant_dids.iter(){

                let mut messages_stream = user.stream_records::<BmailMessageRecord>(participant, "app.bsky.profile", 100, false).await?;
                let mut stream_output = Vec::new();
                while let Ok(record) = messages_stream.next().await {
                    stream_output.push(record);
                }
                // 1.1. Filter by Conversation ID
                stream_output.drain_filter(|r| r.value.bmail_conversation_id != self.conversation_id);
                // 1.2. Drop/Drain any that are older than the latest for each participant
                let latest_post = self.recipient_active_time.get(&participant.to_string());
                if let Some(latest_post) = latest_post{
                    stream_output.drain_filter(|r| &r.value.bmail_created_at <= latest_post);
                }
                // 1.3 Add Them to the Mixer Map
                for record in stream_output.into_iter(){
                    let d_msg = decrypt_and_decode(identity, &record.value.bmail_cipher_text).await?;
                    insert_with_collisions(&mut self.messages, &d_msg);

                }

            }
            //2. Drain mixer_map into conversation
            mixer_map.into_iter().for_each(|(_k, v)| insert_with_collisions(&mut self.messages, &v));
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
impl MessageKey{
    pub fn new() -> Self{
        Self{
            created_at: Utc::now(),
            count: 0
        }
    }
    pub fn new_with_count(count: u32) -> Self{
        Self{
            created_at: Utc::now(),
            count,
        }
    }
    pub fn update_count(&mut self, count: u32) -> &Self{
       self.count = count;
       self
    }
}

/// Record type that can encoded for a Bmail Message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BmailMessageRecord {
    /// The type of the Record for Bluesky. We don't really use it, but we might later
    #[serde(rename(serialize = "$type", deserialize = "$type"))]
    pub bmail_created_at: DateTime<Utc>,
    pub bmail_conversation_id: Uuid,
    pub bmail_cipher_text: String,
    pub bmail_type: String,
    pub bmail_creator: String,
    pub bmail_version: usize,
    pub bmail_recipients: Vec<String>,
}

impl BmailMessageRecord {
    /// Convert a Bmail message from a BmailMessageRecord into a Message struct
    pub async fn into_decrypted_message(
        &self,
        identity: &Identity,
    ) -> Result<DecryptedMessage, BmailError> {

        let binary_message=decrypt_and_decode(identity, &self.bmail_cipher_text).await?;

        Ok(DecryptedMessage {
            created_at: self.bmail_created_at,
            creator: self.bmail_creator.clone(),
            conversation_id: self.bmail_conversation_id,
            message: String::from_utf8(binary_message).map_err(BmailError::FromStringError)?,
            recipients: self.bmail_recipients.clone(),
            version: self.bmail_version,
        })
    }
}
//Data structure for a single Bmail
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecryptedMessage {
    pub created_at: DateTime<Utc>,
    pub creator: String,
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
        })
    }
}

/// Record type that can be passed to create_record() to store identity info(public_key) about the sender in a user profile.
/// Probably wrapped in Record<BmailEnbaledProfile>
#[derive(Debug, Serialize, Deserialize)]
pub struct BmailEnabledProfile {
    #[serde(rename(deserialize = "$type", serialize = "$type"))]
    pub rust_type: Option<String>,
    pub avatar: Option<Blob>,
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
pub fn insert_with_collisions(map: &mut BTreeMap<MessageKey, DecryptedMessage>, msg: &DecryptedMessage){
    let mut count = 0;
    let mut key = MessageKey::new_with_count(count);
    loop {
        if !map.contains_key(&key){
            map.insert(key.clone(), msg.clone());
            return
        }
        count+=1;
        key.update_count(count);
    }
}

/// The types of things the Firehose might send and be returned from the Firehose thread
#[derive(Debug, Clone)]
pub enum FirehoseMessages{
    Bmail(BmailMessageRecord),
    BmailLike(BmailLike)
}
