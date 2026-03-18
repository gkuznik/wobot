use std::collections::HashMap;
use std::sync::atomic::AtomicU64;

use nonempty::NonEmpty;
use poise::serenity_prelude::{ChannelId, Colour, GuildId, ReactionType, UserId};
use serde::Deserialize;
use tokio::sync::Mutex;

#[derive(Debug, Deserialize)]
pub(crate) enum ReplyKind {
    Embed {
        title: String,
        description: String,
        user: UserId,
        #[serde(default)]
        ping: bool,
        #[serde(default)]
        /// colour as an integer
        colour: Colour,
    },
    Message(String),
    RandomMessage(NonEmpty<String>),
}

#[derive(Debug, Deserialize)]
pub(crate) struct AutoReply {
    pub(crate) keywords: Vec<String>,
    pub(crate) kind: ReplyKind,
    pub(crate) chance: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct LinkFix {
    pub(crate) host: Option<String>,
    pub(crate) tracking: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CeleryConfig {
    pub(crate) prompt: String,
    pub(crate) chance: f64,
    pub(crate) cooldown: u64,
}

#[derive(Debug)]
pub(crate) struct Celery {
    pub(crate) prompt: String,
    pub(crate) chance: f64,
    pub(crate) cooldown: u64,
    pub(crate) counter: AtomicU64,
    pub(crate) mutex: Mutex<()>,
}

pub(crate) fn convert(config: HashMap<ChannelId, CeleryConfig>) -> HashMap<ChannelId, Celery> {
    config
        .into_iter()
        .map(|(k, v)| {
            (
                k,
                Celery {
                    prompt: v.prompt,
                    chance: v.chance,
                    cooldown: v.cooldown,
                    counter: AtomicU64::new(0),
                    mutex: Mutex::new(()),
                },
            )
        })
        .collect()
}

#[derive(Deserialize)]
pub(crate) struct Config {
    #[cfg(feature = "activity")]
    #[serde(default)]
    pub(crate) active_guilds: Vec<GuildId>,
    #[serde(default)]
    pub(crate) event_channel_per_guild: HashMap<GuildId, ChannelId>,
    #[serde(default)]
    pub(crate) link_fixes: HashMap<String, LinkFix>,
    #[serde(default)]
    pub(crate) auto_reactions: HashMap<String, ReactionType>,
    #[serde(default)]
    pub(crate) auto_replies: Vec<AutoReply>,
    #[serde(default)]
    pub(crate) entry_sounds: HashMap<UserId, String>,
    #[serde(default)]
    pub(crate) celery: HashMap<ChannelId, CeleryConfig>,
}

#[cfg(test)]
mod tests {
    use crate::config::{AutoReply, ReplyKind};
    use poise::serenity_prelude::Colour;

    #[test]
    fn parse_autoreply_embed() {
        let test_str = r#"{
            keywords: [
              "wobot info"
              "wobot help"
            ]
            kind: {
              Embed: {
                title: About WoBot
                description: "Hi, I'm **WoBot**!"
                user: 123456
                colour: 15844367
              }
            }
          }"#;
        let auto_reply: AutoReply =
            deser_hjson::from_str(test_str).expect("Failed to parse auto reply");
        assert!(auto_reply.chance.is_none());
        assert_eq!(auto_reply.keywords, vec!["wobot info", "wobot help"]);
        match auto_reply.kind {
            ReplyKind::Embed {
                title,
                description,
                user,
                ping,
                colour,
            } => {
                assert!(!ping);
                assert_eq!(title, "About WoBot");
                assert_eq!(description, "Hi, I'm **WoBot**!");
                assert_eq!(user, 123456);
                assert_eq!(colour, Colour::new(15844367));
            }
            _ => panic!("Wrong ReplyKind"),
        }
    }

    #[test]
    fn parse_autoreply_message() {
        let test_str = r#"{
            keywords: [
              "hello"
            ]
            chance: 0.5
            kind: {
              Message: "Hello, I am WoBot!"
            }
          }"#;
        let auto_reply: AutoReply =
            deser_hjson::from_str(test_str).expect("Failed to parse auto reply");

        assert_eq!(auto_reply.chance, Some(0.5));
        assert_eq!(auto_reply.keywords, vec!["hello"]);

        match auto_reply.kind {
            ReplyKind::Message(msg) => {
                assert_eq!(msg, "Hello, I am WoBot!");
            }
            _ => panic!("Wrong ReplyKind"),
        }
    }

    #[test]
    fn parse_autoreply_random_message() {
        let test_str = r#"{
            keywords: [
              "flip"
              "coin"
            ]
            kind: {
              RandomMessage: [
                "Heads!"
                "Tails!"
              ]
            }
          }"#;
        let auto_reply: AutoReply =
            deser_hjson::from_str(test_str).expect("Failed to parse auto reply");

        assert!(auto_reply.chance.is_none());
        assert_eq!(auto_reply.keywords, vec!["flip", "coin"]);

        match auto_reply.kind {
            ReplyKind::RandomMessage(messages) => {
                let msgs: Vec<_> = messages.into_iter().collect();
                assert_eq!(msgs, vec!["Heads!", "Tails!"]);
            }
            _ => panic!("Wrong ReplyKind"),
        }
    }
}
