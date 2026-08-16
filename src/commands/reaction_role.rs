use crate::commands::utils;
use crate::{Context, Data, UserError, done};
use anyhow::Context as _;
use poise::serenity_prelude;
use poise::serenity_prelude::{
    CacheHttp, ChannelId, Guild, GuildId, Mentionable, Message, MessageId, Reaction,
    ReactionCollector, ReactionType, RoleId,
};
use sqlx::{query, query_as};
use std::collections::VecDeque;
use std::time::Duration;
use tracing::{info, warn};

const REACTION_ROLE_TIMEOUT: Duration = Duration::from_secs(60);

#[poise::command(
    slash_command,
    prefix_command,
    guild_only,
    subcommands("list", "add_easy", "add", "remove")
)]
pub(crate) async fn reaction_role(_ctx: Context<'_>) -> anyhow::Result<()> {
    Ok(())
}

/// Choose the role, then react to the message with the emoji you want to use
#[poise::command(
    slash_command,
    prefix_command,
    required_permissions = "MANAGE_ROLES",
    required_bot_permissions = "MANAGE_ROLES"
)]
pub(crate) async fn add_easy(ctx: Context<'_>, role: RoleId) -> anyhow::Result<()> {
    ctx.defer_ephemeral().await?;
    ctx.say("React to the message with the emoji").await?;

    let guild_id = ctx.guild_id().expect("guild_only");
    let reaction = ReactionCollector::new(ctx)
        .guild_id(guild_id)
        .author_id(ctx.author().id)
        .timeout(REACTION_ROLE_TIMEOUT)
        .await;
    let reaction = match reaction {
        None => {
            ctx.reply("Timeout :(, try again").await?;
            return Ok(());
        }
        Some(r) => r,
    };
    let msg = reaction
        .channel_id
        .message(ctx.http(), reaction.message_id)
        .await?;
    reaction.delete(ctx.http()).await?;
    add_reaction_role(ctx, role, msg, reaction.emoji.clone()).await
}

/// Choose the role, message and emoji for a new reaction role
#[poise::command(
    slash_command,
    prefix_command,
    required_permissions = "MANAGE_ROLES",
    required_bot_permissions = "MANAGE_ROLES"
)]
pub(crate) async fn add(
    ctx: Context<'_>,
    role: RoleId,
    #[description = "Existing Message to react to"] message: Message,
    emoji: ReactionType,
) -> anyhow::Result<()> {
    ctx.defer_ephemeral().await?;
    add_reaction_role(ctx, role, message, emoji).await
}

/// Checks that `ctx.author()` and the bot can both assign `role_id` in the guild.
async fn check_can_assign_role(
    ctx: Context<'_>,
    guild_id: GuildId,
    role_id: RoleId,
) -> anyhow::Result<()> {
    let member = guild_id.member(ctx, ctx.author()).await?;
    let bot_id = ctx.cache().current_user().id;
    let bot_member = guild_id.member(ctx, bot_id).await?;
    let guild = guild_id.to_guild_cached(ctx.cache()).unwrap();

    let role = guild.roles.get(&role_id).ok_or_else(|| {
        UserError::err(format!(
            "Role {} does not exist in this guild",
            role_id.mention()
        ))
    })?;

    if ctx.author().id != guild.owner_id {
        let user_top = Guild::member_highest_role(&guild, &member)
            .ok_or_else(|| UserError::err("You have no roles in this guild"))?;
        if role.position >= user_top.position {
            return Err(UserError::err(format!(
                "You can only assign a role lower than your highest role {}",
                user_top.mention()
            )));
        }
    }

    let bot_top = Guild::member_highest_role(&guild, &bot_member)
        .ok_or_else(|| UserError::err("Bot has no roles in this guild"))?;
    if role.position >= bot_top.position {
        return Err(UserError::err(format!(
            "I cannot assign {} because it is not below my highest role {}",
            role.mention(),
            bot_top.mention()
        )));
    }

    Ok(())
}

async fn add_reaction_role(
    ctx: Context<'_>,
    role_id: RoleId,
    message: Message,
    reaction: ReactionType,
) -> anyhow::Result<()> {
    let emoji = utils::get_emoji_text(&reaction, ctx.data());
    let guild_id = ctx.guild_id().expect("guild_only");
    check_can_assign_role(ctx, guild_id, role_id).await?;

    info!(
        "Adding reaction role {} here {} with emoji {}...",
        role_id,
        message.link(),
        reaction
    );

    query!("INSERT INTO reaction_roles (message_id, channel_id, guild_id, role_id, emoji) VALUES ($1, $2, $3, $4, $5)",
        message.id.get() as i64, message.channel_id.get() as i64, guild_id.get() as i64, role_id.get() as i64, emoji,
    )
        .execute(&ctx.data().database).await
        .context("Adding reaction role failed, is the role/emoji already assigned to this message?")?;

    {
        let mut reaction_roles = ctx.data().reaction_msgs.write().expect("reaction_msgs");
        reaction_roles.insert(message.id.into());
    }

    message.react(ctx.http(), reaction).await?;
    done!(ctx);
}

#[poise::command(
    slash_command,
    prefix_command,
    required_permissions = "MANAGE_ROLES",
    required_bot_permissions = "MANAGE_ROLES"
)]
pub(crate) async fn remove(ctx: Context<'_>) -> anyhow::Result<()> {
    ctx.defer_ephemeral().await?;
    ctx.say("React to the message").await?;

    let guild_id = ctx.guild_id().expect("guild_only");
    let reaction = ReactionCollector::new(ctx)
        .guild_id(guild_id)
        .author_id(ctx.author().id)
        .timeout(REACTION_ROLE_TIMEOUT)
        .await;
    let reaction = match reaction {
        None => {
            ctx.say("Timeout :(, try again").await?;
            return Ok(());
        }
        Some(r) => r,
    };

    reaction.delete_all(ctx.http()).await?;
    remove_reaction_role(ctx, reaction).await
}

async fn remove_reaction_role(ctx: Context<'_>, reaction: Reaction) -> anyhow::Result<()> {
    info!(
        "Removing reaction role here {} with emoji {}...",
        reaction
            .message_id
            .link(reaction.channel_id, reaction.guild_id),
        reaction.emoji
    );
    let emoji = utils::get_emoji_text(&reaction.emoji, ctx.data());
    query!(
        "DELETE FROM reaction_roles WHERE message_id = $1 AND emoji = $2",
        reaction.message_id.get() as i64,
        emoji
    )
    .execute(&ctx.data().database)
    .await?;
    {
        let mut reaction_roles = ctx.data().reaction_msgs.write().expect("reaction_msgs");
        reaction_roles.remove(&reaction.message_id.get());
    }
    done!(ctx);
}

#[poise::command(slash_command, prefix_command, guild_only)]
pub(crate) async fn list(ctx: Context<'_>) -> anyhow::Result<()> {
    let show_all_roles = ctx.framework().options.owners.contains(&ctx.author().id);
    let reaction_roles = if show_all_roles {
        ctx.defer_ephemeral().await?;
        query_as!(ReactionRole, "SELECT * FROM reaction_roles")
            .fetch_all(&ctx.data().database)
            .await?
    } else {
        ctx.defer().await?;
        let guild = ctx.guild_id().expect("guild_only");
        query_as!(
            ReactionRole,
            "SELECT * FROM reaction_roles WHERE guild_id = $1",
            guild.get() as i64
        )
        .fetch_all(&ctx.data().database)
        .await?
    };
    let mut roles = VecDeque::from(["**Message | Emoji | Role**".to_string()]);
    for reaction_role in reaction_roles {
        let guild = Some(GuildId::new(reaction_role.guild_id as u64));
        let channel_id = reaction_role.channel_id as u64;
        let msg_id = reaction_role.message_id as u64;
        roles.push_back(format!(
            "{} {} {}",
            MessageId::new(msg_id).link(ChannelId::new(channel_id), guild),
            reaction_role.emoji,
            RoleId::new(reaction_role.role_id as u64).mention()
        ));
    }
    utils::paginate_text(ctx, &mut roles).await?;
    Ok(())
}

pub(crate) async fn change_reaction_role(
    ctx: &serenity_prelude::Context,
    data: &Data,
    reaction: &Reaction,
    add: bool,
) -> anyhow::Result<()> {
    let has_reaction_role = data
        .reaction_msgs
        .read()
        .unwrap()
        .contains(&reaction.message_id.get());
    if !has_reaction_role || reaction.user(ctx.http()).await?.bot {
        return Ok(());
    }

    let emoji = utils::get_emoji_text(&reaction.emoji, data);
    let reaction_role = query!(
        "SELECT * FROM reaction_roles WHERE message_id = $1 AND emoji = $2",
        reaction.message_id.get() as i64,
        emoji
    )
    .fetch_optional(&data.database)
    .await?;
    if reaction_role.is_none() {
        warn!(
            "Expected reaction role here {} with reaction {}, might be unrelated reaction",
            reaction
                .message_id
                .link(reaction.channel_id, reaction.guild_id),
            reaction.emoji
        );
        return Ok(());
    };
    let record = reaction_role.unwrap();
    let user_id = reaction.user_id.with_context(|| {
        format!(
            "Couldn't get user from reaction {} here {}",
            reaction.emoji,
            reaction
                .message_id
                .link(reaction.channel_id, reaction.guild_id)
        )
    })?;
    let member = GuildId::new(record.guild_id as u64)
        .member(ctx.http(), user_id)
        .await
        .with_context(|| {
            format!(
                "Couldn't get member {} from reaction {} here {}",
                user_id.mention(),
                reaction.emoji,
                reaction
                    .message_id
                    .link(reaction.channel_id, reaction.guild_id)
            )
        })?;

    let role_id = RoleId::new(record.role_id as u64);
    let change = if add {
        member.add_role(ctx.http(), role_id).await
    } else {
        member.remove_role(ctx.http(), role_id).await
    };
    change.with_context(|| {
        let typ = if add { "add" } else { "remove" };
        format!("Couldn't {} role {}", typ, role_id)
    })
}

#[allow(dead_code)]
struct ReactionRole {
    message_id: i64,
    channel_id: i64,
    guild_id: i64,
    role_id: i64,
    emoji: String,
}
