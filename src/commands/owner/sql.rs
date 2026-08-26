use crate::Context;
use itertools::Itertools;
use poise::CreateReply;
use poise::serenity_prelude::{CreateAttachment, CreateEmbed};
use sqlx::{AssertSqlSafe, Column, Row};

fn sql_value_to_string(row: &sqlx::postgres::PgRow, column_index: usize) -> String {
    row.try_get::<Option<String>, usize>(column_index)
        .ok()
        .flatten()
        .or_else(|| {
            row.try_get::<Option<i64>, usize>(column_index)
                .ok()
                .flatten()
                .map(|v| match row.column(column_index).name() {
                    "user_id" => format!("<@{v}>"),
                    "channel_id" => format!("<#{v}>"),
                    _ => v.to_string(),
                })
        })
        .or_else(|| {
            row.try_get::<Option<i32>, usize>(column_index)
                .ok()
                .flatten()
                .map(|v| v.to_string())
        })
        .or_else(|| {
            row.try_get::<Option<i16>, usize>(column_index)
                .ok()
                .flatten()
                .map(|v| v.to_string())
        })
        .or_else(|| {
            row.try_get::<Option<f64>, usize>(column_index)
                .ok()
                .flatten()
                .map(|v| v.to_string())
        })
        .or_else(|| {
            row.try_get::<Option<f32>, usize>(column_index)
                .ok()
                .flatten()
                .map(|v| v.to_string())
        })
        .or_else(|| {
            row.try_get::<Option<bool>, usize>(column_index)
                .ok()
                .flatten()
                .map(|v| v.to_string())
        })
        .or_else(|| {
            row.try_get::<Option<chrono::DateTime<chrono::Utc>>, usize>(column_index)
                .ok()
                .flatten()
                .map(|v| v.to_string())
        })
        .or_else(|| {
            row.try_get::<Option<chrono::NaiveDateTime>, usize>(column_index)
                .ok()
                .flatten()
                .map(|v| v.to_string())
        })
        .or_else(|| {
            row.try_get::<Option<chrono::NaiveDate>, usize>(column_index)
                .ok()
                .flatten()
                .map(|v| v.to_string())
        })
        .or_else(|| {
            row.try_get::<Option<Vec<u8>>, usize>(column_index)
                .ok()
                .flatten()
                .map(|v| format!("BLOB({} bytes)", v.len()))
        })
        .unwrap_or_else(|| " - ".to_string())
}

/// Run a SQL query
#[poise::command(slash_command, prefix_command, owners_only)]
pub(crate) async fn sql(
    ctx: Context<'_>,
    query: String,
    ephemeral: Option<bool>,
) -> anyhow::Result<()> {
    if ephemeral.unwrap_or(true) {
        ctx.defer_ephemeral().await?;
    } else {
        ctx.defer().await?;
    }

    let pool = &ctx.data().database;
    let rows = sqlx::query(AssertSqlSafe(query)).fetch_all(pool).await?;

    if rows.is_empty() {
        ctx.send(
            CreateReply::default().embed(
                CreateEmbed::new()
                    .title("SQL Result")
                    .description("No rows returned."),
            ),
        )
        .await?;
    } else {
        let columns = rows[0].columns();
        let headers = columns.iter().map(|c| c.name()).collect::<Vec<_>>();
        let mut table = String::new();
        table += &headers.join("\t");
        table += "\n";
        for row in &rows {
            let values = (0..columns.len())
                .map(|i| sql_value_to_string(row, i))
                .collect_vec();
            table += &values.join("\t");
            table += "\n";
        }

        let reply = if table.len() <= 4096 {
            CreateReply::default()
                .embed(CreateEmbed::new().title("Query Result").description(table))
        } else {
            CreateReply::default()
                .content(format!("{} rows returned", rows.len()))
                .attachment(CreateAttachment::bytes(table, "result.tsv"))
        };
        ctx.send(reply).await?;
    }
    Ok(())
}
