extern crate core;

use std::collections::{HashMap, HashSet};
use std::env;
use std::fmt::Debug;
use std::fs::read_to_string;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use std::{error::Error, fmt};

use crate::check_birthday::check_birthdays;
use crate::check_reminder::check_reminders;
use crate::commands::*;
#[cfg(feature = "activity")]
use crate::constants::ONE_DAY;
use itertools::Itertools;
#[cfg(feature = "activity")]
use mini_moka::sync::{Cache, CacheBuilder};
use nonempty::NonEmpty;
use poise::builtins::{register_globally, register_in_guild};
use poise::serenity_prelude::{
    ChannelId, ClientBuilder, Colour, GatewayIntents, GuildId, ReactionType, UserId,
};
use poise::{EditTracker, Framework, PrefixFrameworkOptions};
use serde::Deserialize;
use songbird::serenity::SerenityInit;
use sqlx::{PgPool, query};
use tokio::sync::Mutex;
use tracing::info;

mod check_birthday;
mod check_reminder;
mod commands;
mod constants;
mod easy_embed;
mod handler;

#[derive(Debug, Deserialize)]
enum ReplyKind {
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
struct AutoReply {
    keywords: Vec<String>,
    kind: ReplyKind,
    chance: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct LinkFix {
    host: Option<String>,
    tracking: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CeleryConfig {
    prompt: String,
    chance: f64,
    cooldown: u64,
}

#[derive(Debug)]
struct Celery {
    prompt: String,
    chance: f64,
    cooldown: u64,
    counter: AtomicU64,
    mutex: Mutex<()>,
}

fn convert(config: HashMap<ChannelId, CeleryConfig>) -> HashMap<ChannelId, Celery> {
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
struct Config {
    #[cfg(feature = "activity")]
    #[serde(default)]
    active_guilds: Vec<GuildId>,
    #[serde(default)]
    event_channel_per_guild: HashMap<GuildId, ChannelId>,
    #[serde(default)]
    link_fixes: HashMap<String, LinkFix>,
    #[serde(default)]
    auto_reactions: HashMap<String, ReactionType>,
    #[serde(default)]
    auto_replies: Vec<AutoReply>,
    #[serde(default)]
    entry_sounds: HashMap<UserId, String>,
    #[serde(default)]
    celery: HashMap<ChannelId, CeleryConfig>,
}

#[cfg(feature = "activity")]
#[derive(Debug, Clone)]
struct CacheEntry {}

/// User data, which is stored and accessible in all command invocations
#[derive(Debug)]
pub(crate) struct Data {
    cat_api_token: String,
    dog_api_token: String,
    mensaplan_token: String,
    ollama_token: String,
    database: PgPool,
    /// cache used to debounce user activity to once per day
    #[cfg(feature = "activity")]
    activity_per_guild: HashMap<GuildId, Cache<UserId, CacheEntry>>,
    event_channel_per_guild: HashMap<GuildId, ChannelId>,
    link_fixes: HashMap<String, LinkFix>,
    auto_reactions: Vec<(String, ReactionType)>,
    auto_replies: Vec<AutoReply>,
    entry_sounds: HashMap<UserId, String>,
    celery: HashMap<ChannelId, Celery>,
    reaction_msgs: RwLock<HashSet<u64>>,
}

/// error type for user actionable issues like an invalid argument
#[derive(Debug)]
pub struct UserError {
    pub message: String,
}

impl fmt::Display for UserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl Error for UserError {}

impl UserError {
    pub fn err(msg: impl Into<String>) -> anyhow::Error {
        Self {
            message: msg.into(),
        }
        .into()
    }
}

type Context<'a> = poise::Context<'a, Data, anyhow::Error>;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let config_data = read_to_string("assets/config.hjson").unwrap_or_default();
    let config: Config = deser_hjson::from_str(&config_data).expect("Failed to parse config");
    #[cfg(feature = "activity")]
    let activity = config
        .active_guilds
        .into_iter()
        .map(|guild| (guild, CacheBuilder::new(500).time_to_live(ONE_DAY).build()))
        .collect();

    let pool = PgPool::connect(&env::var("DATABASE_URL").expect("DATABASE_URL required"))
        .await
        .expect("Failed to connect to database");

    sqlx::migrate!()
        .run(&pool)
        .await
        .expect("Migrations failed");

    let framework = Framework::builder()
        .options(poise::FrameworkOptions {
            commands: get_all_commands(),
            event_handler: |ctx, event, _framework, data| {
                Box::pin(handler::event_handler(ctx, event, _framework, data))
            },
            on_error: |error| Box::pin(async move { handler::on_error(error).await }),
            prefix_options: PrefixFrameworkOptions {
                prefix: Some("!".to_string()),
                edit_tracker: Some(Arc::from(EditTracker::for_timespan(Duration::from_secs(
                    60,
                )))),
                execute_untracked_edits: true,
                ..Default::default()
            },
            ..Default::default()
        })
        .setup(move |ctx, ready, _framework| {
            Box::pin(async move {
                register_globally(ctx, &[modules(), register_commands()]).await?;
                for guild in &ready.guilds {
                    let modules = get_active_modules(&pool, guild.id).await?;
                    register_in_guild(ctx, &get_active_commands(modules), guild.id).await?;
                    info!("Loaded modules for guild {}", guild.id);
                }
                load_bot_emojis(ctx, ready.guilds.iter().map(|g| g.id).collect_vec()).await?;
                let reaction_msgs: Vec<_> = query!("SELECT message_id FROM reaction_roles")
                    .fetch_all(&pool)
                    .await?;
                info!("Loaded reaction messages");
                check_reminders(ctx.clone(), pool.clone());
                check_birthdays(
                    ctx.clone(),
                    pool.clone(),
                    config.event_channel_per_guild.clone(),
                );
                info!("{} is connected!", ready.user.name);
                Ok(Data {
                    cat_api_token: env::var("CAT_API_TOKEN").unwrap_or_default(),
                    dog_api_token: env::var("DOG_API_TOKEN").unwrap_or_default(),
                    mensaplan_token: env::var("MENSAPLAN_TOKEN").unwrap_or_default(),
                    ollama_token: env::var("OLLAMA_TOKEN").unwrap_or_default(),
                    database: pool,
                    #[cfg(feature = "activity")]
                    activity_per_guild: activity,
                    event_channel_per_guild: config.event_channel_per_guild,
                    link_fixes: config.link_fixes,
                    auto_reactions: config.auto_reactions.into_iter().collect_vec(),
                    auto_replies: config.auto_replies,
                    entry_sounds: config.entry_sounds,
                    celery: convert(config.celery),
                    reaction_msgs: RwLock::new(
                        reaction_msgs
                            .into_iter()
                            .map(|f| f.message_id as u64)
                            .collect(),
                    ),
                })
            })
        })
        .build();

    let discord_token = env::var("DISCORD_TOKEN").expect("DISCORD_TOKEN required");
    let intents = GatewayIntents::non_privileged() | GatewayIntents::MESSAGE_CONTENT;
    let client = ClientBuilder::new(discord_token, intents)
        .framework(framework)
        .register_songbird()
        .await;

    client.unwrap().start().await.unwrap();
}

#[cfg(test)]
mod tests {
    use crate::{AutoReply, ReplyKind};
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
