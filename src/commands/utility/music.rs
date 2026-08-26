use crate::{Context, UserError};
use anyhow::Context as _;
use poise::async_trait;
use poise::serenity_prelude::GuildId;
use songbird::input::core::io::ReadOnlySource;
use songbird::input::{AudioStreamError, ChildContainer, RawAdapter};
use songbird::tracks::TrackHandle;
use songbird::{Event, EventContext, EventHandler as VoiceEventHandler, Songbird, TrackEvent};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use tracing::error;

#[poise::command(
    slash_command,
    prefix_command,
    guild_only,
    subcommands("volume", "play", "stop", "skip")
)]
pub(crate) async fn music(_: Context<'_>) -> anyhow::Result<()> {
    Ok(())
}

static VOLUME: AtomicU8 = AtomicU8::new(50);

async fn get_manager(ctx: &Context<'_>) -> Arc<Songbird> {
    songbird::get(ctx.serenity_context())
        .await
        .expect("Songbird initialized")
        .clone()
}

#[poise::command(slash_command, prefix_command)]
async fn volume(ctx: Context<'_>, #[description = "percent"] volume: u8) -> anyhow::Result<()> {
    if volume == 0 || volume > 200 {
        return Err(UserError::err("Volume must be between 1 and 200"));
    }
    ctx.defer().await?;
    VOLUME.store(volume, Ordering::Relaxed);

    let manager = get_manager(&ctx).await;
    let Some(handler_lock) = manager.get(ctx.guild_id().expect("guild_only")) else {
        return Err(UserError::err("Please join a voice channel first"));
    };

    let handler = handler_lock.lock().await;
    if let Some(track) = handler.queue().current() {
        track.set_volume(volume as f32 / 100.0)?;
    }
    ctx.reply(format!("Volume set to {volume}%")).await?;
    Ok(())
}

#[poise::command(slash_command, prefix_command)]
async fn play(ctx: Context<'_>, url: String) -> anyhow::Result<()> {
    ctx.defer().await?;

    let Some(channel) = ctx
        .guild()
        .expect("guild in cache")
        .voice_states
        .get(&ctx.author().id)
        .and_then(|vs| vs.channel_id)
    else {
        return Err(UserError::err("Please join a voice channel first"));
    };

    let manager = get_manager(&ctx).await;
    let handler_lock = manager
        .join(ctx.guild_id().expect("guild_only"), channel)
        .await
        .context("Failed to join voice channel")?;
    let mut handler = handler_lock.lock().await;

    let mut ytdl = Command::new("yt-dlp")
            .args([
                "-f",
                "bestaudio",
                "-o",
                "-",
                "-q",
                "--no-playlist",
                "--no-part",
                "--no-keep-fragments",
                "--downloader",
                "ffmpeg",
                "--downloader-args",
                "ffmpeg:-nostdin -loglevel quiet -reconnect 1 -reconnect_streamed 1 -reconnect_delay_max 5",
                &url,
            ])
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| AudioStreamError::Fail(Box::new(e)))?;
    let ffmpeg = Command::new("ffmpeg")
        .args([
            "-loglevel",
            "quiet",
            "-i",
            "pipe:0",
            "-f",
            "f32le", // raw f32 PCM little-endian
            "-ar",
            "48000", // 48kHz sample rate (Discord standard)
            "-ac",
            "2", // stereo
            "pipe:1",
        ])
        .stdin(
            ytdl.stdout
                .take()
                .ok_or_else(|| AudioStreamError::Fail("yt-dlp stdout was not piped".into()))?,
        )
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| AudioStreamError::Fail(Box::new(e)))?;
    let source = RawAdapter::new(
        ReadOnlySource::new(ChildContainer(vec![ytdl, ffmpeg])),
        48000, // sample rate
        2,     // stereo
    );

    let song = handler.enqueue_input(source.into()).await;
    track_song(manager, ctx.guild_id().unwrap(), song)?;

    ctx.reply("Added to the queue!").await?;
    Ok(())
}

#[poise::command(slash_command, prefix_command)]
async fn stop(ctx: Context<'_>) -> anyhow::Result<()> {
    ctx.defer().await?;
    let manager = get_manager(&ctx).await;
    manager.remove(ctx.guild_id().expect("guild_only")).await?;
    ctx.reply("See you next time").await?;
    Ok(())
}

#[poise::command(slash_command, prefix_command)]
async fn skip(ctx: Context<'_>) -> anyhow::Result<()> {
    ctx.defer().await?;
    let manager = get_manager(&ctx).await;
    let Some(handler_lock) = manager.get(ctx.guild_id().expect("guild_only")) else {
        return Err(UserError::err("Please join a voice channel first"));
    };
    let handler = handler_lock.lock().await;
    handler.queue().skip()?;
    ctx.reply("Skipped!").await?;
    Ok(())
}

struct SongEndNotifier {
    guild: GuildId,
    manager: Arc<Songbird>,
}

#[async_trait]
impl VoiceEventHandler for SongEndNotifier {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        if let EventContext::Track(track_list) = ctx {
            // this track is not yet removed
            if track_list.len() <= 1
                && let Err(e) = self.manager.remove(self.guild).await
            {
                error!("Failed to leave channel: {:?}", e);
            }
        }
        None
    }
}

pub(crate) fn track_song(
    manager: Arc<Songbird>,
    guild: GuildId,
    song: TrackHandle,
) -> Result<(), anyhow::Error> {
    song.set_volume(VOLUME.load(Ordering::Relaxed) as f32 / 100.0)?;
    song.add_event(
        Event::Track(TrackEvent::End),
        SongEndNotifier { guild, manager },
    )?;
    Ok(())
}
