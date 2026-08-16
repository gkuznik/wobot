use crate::commands::utils::remove_components_but_keep_embeds;
use crate::{Context, UserError};
use poise::CreateReply;
use poise::serenity_prelude::{
    ComponentInteractionCollector, CreateActionRow, CreateButton, CreateInteractionResponse,
    CreateInteractionResponseMessage,
};
use std::borrow::Cow;
use std::process::Command;
use std::time::Duration;
use tracing::{debug, warn};

const MAN_VIEW_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const MSG_MAX_LEN: usize = 2000 - 25; // -25 to account for the title

fn get_pages(input: Cow<str>) -> Vec<String> {
    let lines: Vec<&str> = input.lines().collect();

    if lines.len() <= 2 {
        return vec!["No man page available!".into()];
    }

    let stripped_lines = &lines[1..lines.len() - 1];

    let mut pages = vec![];
    let mut msg: String = "".into();

    for line in stripped_lines {
        // +1 because of newline
        if msg.len() + line.len() + 1 > MSG_MAX_LEN {
            pages.push(msg);
            msg = "".into();
        }

        msg.push_str(line);
        msg.push('\n');
    }

    pages.push(msg);
    pages
}

#[repr(u8)]
#[derive(Copy, Clone, Debug, poise::ChoiceParameter)]
pub(crate) enum Section {
    #[name = "1 Executable programs or shell commands"]
    Executables = 1,
    #[name = "2 System calls (functions provided by the kernel)"]
    SystemCalls = 2,
    #[name = "3 Library calls (functions within program libraries)"]
    LibraryCalls = 3,
    #[name = "4 Special files (usually found in /dev)"]
    SpecialFiles = 4,
    #[name = "5 File formats and conventions, e.g. /etc/passwd"]
    FileFormats = 5,
    #[name = "6 Games"]
    Games = 6,
    #[name = "7 Miscellaneous (including macro packages and conventions), e.g. man(7), groff(7), man-pages(7)"]
    Miscellaneous = 7,
    #[name = "8 System administration commands (usually only for root)"]
    SystemAdministration = 8,
    #[name = "9 Kernel routines [Non standard]"]
    KernelRoutines = 9,
}

/// Consult a man page
#[poise::command(slash_command, prefix_command)]
pub(crate) async fn man(
    ctx: Context<'_>,
    section: Option<Section>,
    page: String,
) -> anyhow::Result<()> {
    ctx.defer().await?;
    debug!("man {:?} {}", section, page);

    // prevent arbitrary file reads
    if !page
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || ".-_+".contains(c))
    {
        return Err(UserError::err("invalid man page"));
    }

    let mut com = Command::new("man");
    com.env_clear().arg("--");
    if let Some(sec) = section {
        com.arg((sec as u8).to_string());
    }
    com.arg(&page);
    let pages = get_pages(String::from_utf8_lossy(&com.output()?.stdout));

    let ctx_id = ctx.id();
    let prev_button_id = format!("{}prev", ctx.id());
    let next_button_id = format!("{}next", ctx.id());

    let mut idx: usize = 0;
    let page_count = pages.len();

    let reply = {
        let components = vec![CreateActionRow::Buttons(vec![
            CreateButton::new(&prev_button_id).emoji('◀'),
            CreateButton::new(&next_button_id).emoji('▶'),
        ])];

        CreateReply::default()
            .components(components)
            .content(format!(
                "**Page {}/{}**```{}```",
                idx + 1,
                page_count,
                pages[idx]
            ))
    };

    let reply_handle = ctx.send(reply).await?;

    while let Some(press) = ComponentInteractionCollector::new(ctx)
        .channel_id(ctx.channel_id())
        .filter(move |press| press.data.custom_id.starts_with(&ctx_id.to_string()))
        .timeout(MAN_VIEW_TIMEOUT)
        .await
    {
        if press.data.custom_id == next_button_id {
            idx += 1;
            if idx >= page_count {
                idx = 0;
            }
        } else if press.data.custom_id == prev_button_id {
            if idx > 0 {
                idx -= 1;
            } else {
                idx = page_count - 1;
            }
        } else {
            // This is an unrelated button interaction
            warn!(
                "unrelated button interaction with same ctx id: {:?}",
                press.data
            );
            continue;
        }

        // Update the message with the new page contents
        let message = CreateInteractionResponseMessage::default().content(format!(
            "**Page {}/{}**```{}```",
            idx + 1,
            page_count,
            pages[idx]
        ));

        press
            .create_response(
                ctx.http(),
                CreateInteractionResponse::UpdateMessage(message),
            )
            .await?;
    }
    remove_components_but_keep_embeds(ctx, CreateReply::default(), reply_handle).await
}
