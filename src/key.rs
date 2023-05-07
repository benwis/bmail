use std::io::{Read, Write};
use std::{path::PathBuf, str::FromStr};
use age::x25519::Identity;
use std::fs::OpenOptions;
use age::secrecy::ExposeSecret;

use crate::errors::BmailError;

/// Get the identity for a recipient or sender other than me from their profile info
pub fn get_user_key_from_profile() -> Result<String, BmailError>{
//TODO: Parse profile for identity
Ok(String::new())
}

/// Attempt to read saved identity from file or generate a new one for the user of the app
pub fn get_identity(path: &PathBuf) -> Result<Identity, BmailError>{
    let mut f = OpenOptions::new()
    .read(true)
    .write(true)
    .create(true)
    .open(path).map_err::<BmailError, _>(Into::into)?;

    let mut privkey = String::new();
    let _file_size = f.read_to_string(&mut privkey).map_err::<BmailError, _>(Into::into)?;
    let identity;
    //If the string was empty, we're going to create it and then write it to the file
    if privkey.is_empty(){
        identity = age::x25519::Identity::generate();
        let id_string = identity.to_string();
        let secret = id_string.expose_secret();
        f.write_all(secret.as_bytes())?;

    } else{
        identity = Identity::from_str(&privkey).unwrap();
    }
    Ok(identity)

}