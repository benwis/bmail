# Bmail

An unofficial Proof of Concept encrypted DM system for Bluesky

## Security Warning and Disclaimer

I made this project as a Proof of Concept, and as a neat hobby tool. I am not a cryptography engineer, and this has not undergone an audit.
While I spent effort to make it safe and effective, it could delete all your data, expose all your secrets, or crash your computer.
Check out the [Security Model](#security-model-and-assumptions) section for more info. USE AT YOUR OWN RISK.

## Installation

1. Copy the `bmail.toml.example` file to `bmail.toml`. It expects this file to be present in the same directory as the bmail binary.

2. Add your handle and an app password to the handle and password fields in quotes. You need to include your full handle, so either your custom domain or handle.bsky.social

```toml
[user]
handle="example.bsky.social"
password="an-app-password"

[key]
file_path="keys/bmail_identity.secret"
```

3. Run the binary. If you're a developer, you'll need to have Nightly Rust installed to compile it

## Technical Details

This is an example of an on repo direct-messaging system. Messages you send are encrypted and stored in your repo as profile records. Anyone can see the encrypted messages, and who you're messaging, but will be unable to read them without the intended recipient's private key.

### Encryption

Messages are encrypted with the Rust implementation of Age called [Rage](https://github.com/str4d/rage). It was designed to encrypt files, not for encrypted chat.
This leads to several deficiencies.
1. No Key Rotation. If you lose your private key, you lose your messages
2. No Sender Validation. If someone edits your public key on your profile, whether by hacking in or being a malicious Bluesky server, they can pretend to be you. This is called a Man in the Middle Attack. This does not expose your private key though, so old messages are safe

### New Messages

If the Bmail app is running, it will scan the Firehose for new Bmail Messages, find ones that you are involved in, decrypt them, and show them to you.

If the Bmail app is not running, and you load a conversation, it will scan the participant's profiles for all the messages in the conversation, decrypt them, and show them to you.

### Key Exchange

When you start the app, a public/private keypair is generated for your computer. The private key is stored in the keys folder. Do not lose this key, as it is required to decrypt messages sent to you. If you want to run this on multiple clients, you'll need to copy the key to each client.

Your public key is attached to your Bluesky profile as a field on your profile record. Others will encrypt messages to you with your public key. For this to work, you must trust that your PDS provider(currently Bluesky) will not change your public key. If someone does, they will be able to decrypt future messages. When you send a message to someone, it'll scan their profile for their public key. If it finds it, it will encrypt your message with it. If it doesn't, it will throw an error.

Currently, there is no way to rotate keys, and you are trusting your personal data store to present your public key accurately. However, since you hold your private key, the best a malicious PDS or  Bluesky account hacker would be able to do is impersonate you in the future.

If you suspect this is the case, you can delete or move your key file, and Bmail will generate a new public/private key pair for future messages

### Notifications

When the app is started for the first time, it will create a post that will be hidden from your timeline with the message "You've got Bmail". When you receive a Bmail, the sender will like that post. In typical clients, you won't be able to tell which conversation has a new message, just who sent the new message. There is a custom field on that Like that indicates which Conversation it is, but that is only visible in dev tools right now.

## Security Model and Assumptions
1. This trusts your PDS, currently only Bluesky, to present your public key accurately. This means you trust the Bluesky team or your server admins. They could impersonate you in the future.
2. Currently all Bmail messages are stored in your account, and are readable by anyone. They cannot be deleted(since deletion doesn't actually delete them). If someone were able to crack Age encryption(very unlikely), or steal your private key(more likely), they would be able to read all messages you have ever sent with that public/private keypair. If they could do that sneakily, they could eavesdrop on all your future conversations with that keypair.
3. Message metadata is not encrypted and easily queryable. Anyone can see who is messaging whom, when and how many messages were sent. This is a limitation of using the Bluesky repo as the transport medium.
4. This has received no audits, and I am not a security/cryptography engineer. It's quite possible that I have implement this incorrectly. I did use a prebuilt cryptography library, so the risk is lesser, but it still exists. That library, also, has not received a security audit.


## FAQ
1. Why did you make this?
	- It seemed like a neat hobby project, and an excuse to learn more about Rust and build out the Bisky Bluesky client library. Bluesky hadn't done it yet, and I thought it might be fun to message some of my friends
2. Is it perfect?
	- Definitely not, besides the things I don't know about, it has some rough edges. See the TODO section below for details
3. Will you keep working on this?
	- If there's interest, and the Bluesky team doesn't release their own.(I'm sure they will).
4. Why did you choose Age encryption instead of something like libSignal? Don't you know Age is supposed to be for encrypting files?
	- Since I'm not a cryptography engineer, I wanted something that was documented, had a Rust crate, and was fairly easy to use. Age fits the bill. It has several deficiencies compared to libsignal, but it should provide adequate encryption. libsignal is great, but is complicated and not documented.
5. Why do direct messages on repo? Paul Frazee has said they should be off repo.
	- When I was doing the system design for this, I had two primary restrictions.
		1. You should be able to message others when one or more of the other parties is not online.
			- This prevented me from doing person to person encrypted tunnels with something like the Noise. You'd only be able to send messages if the other person is online, if they're not you'd have to leave the app open and continuously poll for a connection. and you'd have to advertise your address somehow, unless there is a third party. Which brings us to the next restriction
		2.  I do not want to run any servers or store any of your messages
			- I have no interest in holding anybody's secrets, routing secret connections, or worrying about any of those details. If I was, I could run an intermediate server to connect people through encrypted channels, or store messages for delivery to recipients later. 
			
## TODO
1. Messages do not wrap. Both writing new messages and receiving long messages may overflow the messages box
2. Key Rotation. There is no way to rotate keys without losing all your messages
3. Better Conversation Selection. Make it so it stores a list of your conversations, so you don't have to remember the participants
4. Store more info locally. Most things are queried each time, despite them being unlikely to change
5. Firehose message parsing is slow. Not sure why yet, but it takes several seconds for a Firehose message to appear in the UI
6. The structure for notifications are there, but currently they do nothing in the app itself
7. Group conversations. While technically possible, I haven't tried them yet. They might not work.
8. No API rate limiting. You could create a conversation with hundreds of recipients, and then spam Bluesky with messages
9. All Times are in UTC