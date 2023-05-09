use age::secrecy::ExposeSecret;
use age::x25519::{Identity, Recipient};
use bisky::lexicon::com::atproto::repo::Record;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::{path::PathBuf, str::FromStr};

use crate::errors::BmailError;
use crate::message::BmailEnabledProfile;

/// Attempt to read saved identity from file or generate a new one for the user of the app
pub fn get_identity(path: &PathBuf) -> Result<Identity, BmailError> {
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path)
        .map_err::<BmailError, _>(Into::into)?;

    let mut privkey = String::new();
    let _file_size = f
        .read_to_string(&mut privkey)
        .map_err::<BmailError, _>(Into::into)?;
    let identity;
    //If the string was empty, we're going to create it and then write it to the file
    if privkey.is_empty() {
        identity = age::x25519::Identity::generate();
        let id_string = identity.to_string();
        let secret = id_string.expose_secret();
        f.write_all(secret.as_bytes())?;
    } else {
        identity = Identity::from_str(&privkey).unwrap();
    }
    Ok(identity)
}
// /// Process Recipient Identity from a an actor.profile Record
// pub fn process_profile_records_for_identity(records: Vec<Record<BmailInfo>>) -> Result<Recipient, BmailError>{
//     let mut pub_key = None;
//     for record in records{
//         if record.value.bmail_type == "bmail_pubkey" && pub_key.is_none(){
//             pub_key = match record.value.bmail_pub_key{
//                 Some(k) => {
//                     println!("Found Public Key: {}", &k); 
//                     Some(k)
//                 },
//                 None => return Err(BmailError::MissingRecipientIdentity)
//             };
//         } else if record.value.bmail_type == "bmail_pubkey" && pub_key.is_some(){
//             // Which should we use Bob? Gee, I don't know. Somebody fucked up
//             return Err(BmailError::MultipleRecipientKeys)
//         }
//     }
 
//     if let Some(pub_key) = pub_key{
//     let recipient =
//         Recipient::from_str(&pub_key).map_err(|_| BmailError::ParseRecipientError)?;
//     Ok(recipient)
//     } else {
//         Err(BmailError::MissingRecipientIdentity)
//     }

// }
