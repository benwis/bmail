use std::collections::HashMap;
use chrono::{Utc, DateTime};
/// Hashmap with DID Key of recipient and Vec of Message Types 
///Hashmap<String, Vec<String>>

#[derive(Default)]
pub struct Conversations(pub HashMap<String, Vec<Message>>);

impl Conversations{
    /// Add a message to the messages storage method
    pub fn add(&mut self, command: &str, sender_did: &str, recipient_did: &str, raw_message: &str, message: &str, created_at: &DateTime<Utc>){
        let message_struct = Message{
            created_at: created_at.to_owned(),
            message: Some(message.to_string()),
            command: command.to_string(),
            sender_did: sender_did.to_string(),
            recipient_did: recipient_did.to_string(),
            raw_message: raw_message.to_string(),

        };
        match self.0.get_mut(sender_did){
            Some(r) => r.push(message_struct),
            None => {
                let msgs = vec![message_struct];
                self.0.insert(sender_did.to_string(), msgs);
            }
        };

    }
}


//Contains information about the received message
pub struct Message {
    pub command: String,
    pub sender_did: String, 
    pub recipient_did: String, 
    pub created_at: DateTime<Utc>, 
    pub raw_message: String,
    pub message: Option<String>
}