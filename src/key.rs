use age::secrecy::ExposeSecret;
use age::{
    x25519::{Identity, Recipient},
    Recipient as RecipientTrait,
};
use base64::engine::general_purpose;
use base64::Engine;
use bisky::lexicon::com::atproto::repo::Record;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::{path::PathBuf, str::FromStr};

use crate::errors::BmailError;
use crate::message::BmailEnabledProfile;
use crate::SharableBluesky;

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

/// Scrape the recipient's Profile for their Public Key so we can encrypt this thing
pub async fn get_recipient_for_bskyer(
    bsky: SharableBluesky,
    handle: &str,
) -> Result<(Option<Recipient>, Record<BmailEnabledProfile>), BmailError> {
    let mut bsky = bsky.0.write().await;
    let mut user = bsky.user(handle)?;

    let profile_record = user
        .get_record::<BmailEnabledProfile>(handle, "app.bsky.actor.profile", "self")
        .await?;
    let recipient = match &profile_record.value.bmail_pub_key {
        Some(k) => Some(Recipient::from_str(k).map_err(|_| BmailError::ParseRecipientError)?),
        None => None,
    };
    Ok((recipient, profile_record))
}

/// CBORify, Encrypt with age, and base64 encode some data to be passed around to certain recipients
pub async fn encrypt_and_encode<T>(
    recipients: Vec<Box<dyn RecipientTrait + Send>>,
    payload: T,
) -> Result<String, BmailError>
where
    T: Serialize,
{
    //Stores cbored data
    let mut cbor_buffer: Vec<u8> = Vec::new();
    // Write payload into cbor_futter as cbor
    ciborium::ser::into_writer(&payload, &mut cbor_buffer)?;
    // Encrypt the plaintext to a ciphertext...
    let encryptor = age::Encryptor::with_recipients(recipients).expect("we provided a recipient");

    let mut encrypted = vec![];
    let mut writer = encryptor
        .wrap_output(&mut encrypted)
        .map_err::<BmailError, _>(Into::into)?;
    writer.write_all(cbor_buffer.as_slice())?;
    writer.finish()?;

    let encoded: String = general_purpose::STANDARD_NO_PAD.encode(&encrypted);
    Ok(encoded)
}

/// CBORify and base64 encode some data to fit in JS requirements
pub async fn encode<T>(
    payload: T,
) -> Result<String, BmailError>
where
    T: Serialize,
{
    //Stores cbored data
    let mut cbor_buffer: Vec<u8> = Vec::new();
    // Write payload into cbor_futter as cbor
    ciborium::ser::into_writer(&payload, &mut cbor_buffer)?;

    let encoded: String = general_purpose::STANDARD_NO_PAD.encode(&cbor_buffer);
    Ok(encoded)
}
/// base64 decode, Decrypt with private key, and then decode from CBOR some data
pub async fn decrypt_and_decode<T>(identity: &Identity, payload: &str) -> Result<T, BmailError>
where
    T: DeserializeOwned,
{
    let decoded = general_purpose::STANDARD_NO_PAD.decode(payload)?;

    // Decrypt Binary data
    let decrypted = {
        let decryptor =
            match age::Decryptor::new(decoded.as_slice()).map_err::<BmailError, _>(Into::into)? {
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
    Ok(ciborium::de::from_reader(decrypted.as_slice())?)
}
/// base64 decode, and then decode from CBOR some data
pub async fn decode<T>(payload: &str) -> Result<T, BmailError>
where
    T: DeserializeOwned,
{
    let decoded = general_purpose::STANDARD_NO_PAD.decode(payload)?;

    Ok(ciborium::de::from_reader(decoded.as_slice())?)
}
