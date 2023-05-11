use crate::{errors::BmailError, key::get_recipient_for_bskyer, SharableBluesky};
use age::{
    x25519::{Identity, Recipient},
    Recipient as RecipientTrait,
};
use base64::{engine::general_purpose, Engine as _};
use bisky::lexicon::com::atproto::repo::{Blob, StrongRef};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    collections::HashMap,
    io::{Read, Write},
};
use uuid::Uuid;
pub struct Conversation {
    /// A Unique ID for the Conversation this is a part of, to make it easier for clients to poll a particular conversation. Multiple Records might have the same ID, this means they are participants of the same chain
    pub conversation_id: Uuid,
    pub messages: BTreeMap<DateTime<Utc>, DecryptedMessage>,
    /// The currently active index for each recipient as seen by this client
    /// This way, we can filter out all the processed messages.
    /// Keyed by DID and contains an BTreeMap index
    pub recipient_active_index: HashMap<String, usize>,
    /// The participants in a Conversation. Used so we know whose accounts to try to find messages on
    pub participants: Vec<String>,
}

impl Conversation {
    //     /// Join together different peoples ConversationPortions for a specific Conversation ID into a Conversation
    //     /// that can be displayed by the UI and stored locally
    //     pub fn stitch_conversation_portions(&mut self, c_portions: Vec<ConversationPortion>) {
    //         // Get all Profile Records as ConversationPortions, filter based on conversation id and whether they are participant in the conversation
    //         let relevant_portions: Vec<&ConversationPortion> = c_portions
    //             .iter()
    //             .filter(|&cp| {
    //                 self.participants.contains_key(&cp.owner)
    //                     && Some(self.bmail_conversation_id) == cp.bmail_conversation_id
    //             })
    //             .collect();
    //         let mut mixer_map = BTreeMap::new();

    //         // 1. Get Recipient DIDs from conversation portions
    //         // 2. Get Last Block
    //         // Filter Portions based on last seen Portion index
    //         // Find timestamp of last seen message in active portion. Split messages Vec with new messages
    //         // If there are more blocks after the last seen one, append them to the buffer

    //         mixer_map.drain(RangeFull).for_each(|v| {
    //             &mut self.messages.insert(v.0, v.1);
    //         });
    //     }
}

/// Keeps track of the messages seen on this client
pub struct MyConversationPortion {
    pub conversation_id: Uuid,
    pub messages: BTreeMap<DateTime<Utc>, DecryptedMessage>,
}
// /// A data structure to hold a conversation between a group of individuals
// /// Stored in an bsky.actor.profile Record, clients will find it by parsing the id from
// /// the notification of the like of their notification post.
// #[derive(Debug, Serialize, Deserialize, Default)]
// pub struct ConversationPortion {
//     /// The type of the Record for Bluesky. We don't really use it, but we might later
//     #[serde(rename(serialize = "$type", deserialize = "$type"))]
//     pub r#type: String,
//     /// The DID of the owner of this ConversationPortion
//     pub owner: String,
//     /// The time the conversation was created
//     #[serde(rename(serialize = "createdAt", deserialize = "createdAt"))]
//     pub bmail_created_at: Option<DateTime<Utc>>,
//     /// A Unique ID for the Conversation this is a part of, to make it easier for clients to poll a particular conversation. Multiple Records might have the same ID, this means they are participants of the same chain
//     pub bmail_conversation_id: Option<Uuid>,
//     /// Indicates, in sequential order, which block of messages this ConversationPortion represents. This is done to avoid the 100k request limit for Bsky
//     pub bmail_conversation_portion_index: Option<usize>,
//     /// The messages in a Conversation. Keyed by the DIDs of the participants and the
//     pub bmail_messages: Option<BTreeMap<DateTime<Utc>, Message>>,
//     /// The participants in a ConversationPortion. Keyed by the DIDs of the participants, the value is the bmail index for when they were added
//     pub participants: Option<HashMap<String, u128>>,
// }
// impl ConversationPortion {
//     /// Create a new Conversation between some recipients(indicated by DID)
//     pub fn new(&mut self, owner: &str, recipients: Vec<String>) -> Self {
//         // Create a record of which messages are encoded for which recipients. New recipients can be added later, but will require each participant
//         // to update their recipient keys to include them.
//         let og_participants = recipients.iter().map(|r| (r.to_string(), 0)).collect();
//         Self {
//             r#type: "bsky.actor.profile".to_string(),
//             bmail_created_at: Some(Utc::now()),
//             bmail_conversation_id: Some(Uuid::new_v4()),
//             bmail_messages: Some(BTreeMap::new()),
//             participants: Some(og_participants),
//             bmail_conversation_portion_index: Some(0),
//             owner: owner.to_string(),
//         }
//     }
//     /// Add a message to the ConversationPortion bmail_messages field
//     pub fn add_message(&mut self, msg: &Message) {
//         if let Some(messages) = &mut self.bmail_messages {
//             messages.insert(Utc::now(), msg.to_owned());
//         }
//     }
// }

// //Data structure for a Bmail Message key
// #[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
// pub struct MessageKey {
//     pub created_at: DateTime<Utc>,
//     pub count: u32,

// }
// impl MessageKey{
//     pub fn new() -> Self{
//         Self{
//             created_at: Utc::now(),
//             count: 0
//         }
//     }
//     pub fn new_with_count(count: u32) -> Self{
//         Self{
//             created_at: Utc::now(),
//             count,
//         }
//     }
//     pub fn update_count(&mut self, count: u32) -> &Self{
//        self.count = count;
//        self
//     }
// }

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
    pub fn into_decrypted_message(
        &self,
        creator: &str,
        identity: &Identity,
    ) -> Result<DecryptedMessage, BmailError> {
        // Decode Base64 encoded cipher text into Binary data
        let decoded = general_purpose::STANDARD_NO_PAD.decode(&self.bmail_cipher_text)?;

        // Decrypt Binary data
        let decrypted = {
            let decryptor = match age::Decryptor::new(decoded.as_slice())
                .map_err::<BmailError, _>(Into::into)?
            {
                age::Decryptor::Recipients(d) => d,
                _ => unreachable!(),
            };

            let mut decrypted = vec![];
            let mut reader = decryptor
                .decrypt(std::iter::once(identity as &dyn age::Identity))
                .map_err::<BmailError, _>(Into::into)?;
            reader
                .read_to_end(&mut decrypted)
                .map_err::<BmailError, _>(Into::into)?;

            decrypted
        };

        Ok(DecryptedMessage {
            created_at: self.bmail_created_at,
            creator: creator.to_string(),
            conversation_id: self.bmail_conversation_id,
            message: String::from_utf8(decrypted).map_err(BmailError::FromStringError)?,
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
        identity: &Identity,
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
        // Encrypt the plaintext to a ciphertext...
        let encryptor =
            age::Encryptor::with_recipients(recipient_keys).expect("we provided a recipient");

        let mut encrypted = vec![];
        let mut writer = encryptor
            .wrap_output(&mut encrypted)
            .map_err::<BmailError, _>(Into::into)?;
        writer.write_all(self.message.as_bytes())?;
        writer.finish()?;
        println!("Number of Bytes: {}", encrypted.len());

        let encoded: String = general_purpose::STANDARD_NO_PAD.encode(&encrypted);

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

#[derive(Debug, Serialize, Deserialize)]
pub struct BmailLike {
    #[serde(rename(deserialize = "createdAt"))]
    #[serde(rename(serialize = "createdAt"))]
    pub created_at: DateTime<Utc>,
    pub subject: StrongRef,
    pub bmail_recipients: Vec<String>,
    pub bmail_conversation_id: Uuid,
}

// /// Function that can be recursed over to insert into a BTreeMap with possible collisions
// pub fn insert_with_collisions(map: &mut BTreeMap<DateTime<Utc>, Message>, msg: &Message){
//     let mut count = 0;
//     let mut key = MessageKey::new_with_count(count);
//     loop {

//         if !map.contains_key(&key){
//             map.insert(key.clone(), msg.clone());
//             return
//         }

//         count+=1;
//         key.update_count(count);
//     }
// }
