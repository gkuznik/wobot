extern crate core;

use std::collections::{HashMap, HashSet};
use std::env;
use std::fmt::Debug;
use std::fs::read_to_string;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use std::{error::Error, fmt};

use crate::check_birthday::check_birthdays;
use crate::check_reminder::check_reminders;
use crate::commands::*;
use crate::config::{AutoReply, Celery, Config, LinkFix, convert};
#[cfg(feature = "activity")]
use crate::constants::ONE_DAY;
use itertools::Itertools;
#[cfg(feature = "activity")]
use mini_moka::sync::{Cache, CacheBuilder};
use poise::builtins::{register_globally, register_in_guild};
use poise::serenity_prelude::{
    ChannelId, ClientBuilder, GatewayIntents, GuildId, ReactionType, UserId,
};
use poise::{EditTracker, Framework, PrefixFrameworkOptions};
use songbird::serenity::SerenityInit;
use sqlx::{PgPool, query};
use tracing::info;

mod check_birthday;
mod check_reminder;
mod commands;
mod config;
mod constants;
mod easy_embed;
mod handler;

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
