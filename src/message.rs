use bisky::lexicon::com::atproto::repo::Blob;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::collections::HashMap;

/// A data structure to hold a conversation between a group of individuals
/// Stored in an bsky.actor.profile Record, clients will find it by parsing the id from
/// the notification of the like of their notification post.  
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ConversationPortion {
    /// The type of the Record for Bluesky
    #[serde(rename(serialize = "$type", deserialize = "$type"))]
    pub r#type: String,
    /// The time the conversation was created
    #[serde(rename(serialize = "createdAt", deserialize = "createdAt"))]
    pub bmail_created_at: Option<DateTime<Utc>>,
    /// A Unique ID for the conversation, to make it easier for clients to poll a particular conversation. Multiple Records might have the same ID, this means they are participants of the same chain
    pub bmail_conversation_id: Option<Uuid>,
    /// Indicates, in sequential order, which block of messages this Conversation represents. This is done to avoid the 100k request limit for Bsky
    pub bmail_conversation_index: Option<u64>,
    /// The messages in a Conversation. Keyed by the DIDs of the participants and the
    pub bmail_messages: Option<Vec<Message>>,
    /// The participants in a Conversation. Keyed by the DIDs of the participants, the value is the bmail index for when they were added 
    pub participants: Option<HashMap<String, u128>>,
}

impl ConversationPortion {
    /// Create a new Conversation between some recipients(indicated by DID)
    pub fn new(&mut self, recipients: Vec<String>) -> Self {
        // Create a record of which messages are encoded for which recipients. New recipients can be added later, but will require each participant
        // to update their recipient keys to include them.
        let og_participants = recipients.iter().map(|r| (r.to_string(), 0)).collect();
        Self {
            r#type: "bsky.actor.profile".to_string(),
            bmail_created_at: Some(Utc::now()),
            bmail_conversation_id: Some(Uuid::new_v4()),
            bmail_messages: Some(Vec::new()),
            bmail_conversation_index: Some(0),
            participants: Some(og_participants),
        }
    }
    /// Add a message to the Conversation
    pub fn add_message(&mut self, msg: &Message) {
        if let Some(conversation) = &mut self.bmail_messages {
            conversation.push(msg.clone());
        }
    }
}

//Data structure for a single Bmail
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub created_at: DateTime<Utc>,
    pub creator: String,
    pub raw_message: String,
    pub message: String,
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
}
