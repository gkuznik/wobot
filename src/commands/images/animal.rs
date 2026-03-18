use std::ops::Range;

use poise::serenity_prelude::CreateEmbed;
use poise::{CreateReply, command};
#[cfg(not(test))]
use rand::{RngExt, rng};
use serde::Deserialize;

use crate::Context;
use crate::constants::HTTP_CLIENT;

/// random animal, possible: Fox, Cat, Dog
#[command(slash_command, prefix_command)]
pub async fn floof(ctx: Context<'_>) -> anyhow::Result<()> {
    ctx.defer().await?;

    let image = get_random_image(&ctx.data().cat_api_token, &ctx.data().dog_api_token).await?;
    let embed = CreateEmbed::default().title("Have a floof :)").image(image);
    ctx.send(CreateReply::default().embed(embed)).await?;
    Ok(())
}

const API_RANGE: Range<i32> = 0..6;
async fn get_random_image(cat_api_token: &str, dog_api_token: &str) -> anyhow::Result<String> {
    #[cfg(test)]
    let api_choice = tests::API_CHOICE.load(std::sync::atomic::Ordering::SeqCst);
    #[cfg(not(test))]
    let api_choice = rng().random_range(API_RANGE);

    match api_choice {
        0 => do_json::<TopLevelImage>("https://randomfox.ca/floof/").await,
        1 => do_json::<TopLevelURL>("https://random.dog/woof.json").await,
        2 => do_json::<DogCEO>("https://dog.ceo/api/breeds/image/random").await,
        3 => {
            let url = format!("https://api.thecatapi.com/v1/images/search?api_key={cat_api_token}");
            do_json::<TheCatAPI>(&url).await
        }
        4 => {
            let url = format!("https://api.thedogapi.com/v1/images/search?api_key={dog_api_token}");
            do_json::<TheCatAPI>(&url).await
        }
        5 => do_json::<TopLevelURL>("https://cataas.com/cat?json=true").await,
        _ => unreachable!(),
    }
}

async fn do_json<T: AnimalImage + for<'de> Deserialize<'de>>(url: &str) -> anyhow::Result<String> {
    let response = HTTP_CLIENT.get(url).send().await?;
    Ok(response.json::<T>().await?.extract_url())
}

trait AnimalImage {
    fn extract_url(self) -> String;
}

#[derive(Deserialize)]
struct TopLevelURL {
    url: String,
}

impl AnimalImage for TopLevelURL {
    fn extract_url(self) -> String {
        self.url
    }
}

#[derive(Deserialize)]
struct TopLevelImage {
    image: String,
}

impl AnimalImage for TopLevelImage {
    fn extract_url(self) -> String {
        self.image
    }
}

#[derive(Deserialize)]
struct DogCEO {
    message: String,
}

impl AnimalImage for DogCEO {
    fn extract_url(self) -> String {
        self.message
    }
}

#[derive(Deserialize)]
struct TheCatAPI {
    data: TopLevelURL,
}

impl AnimalImage for TheCatAPI {
    fn extract_url(self) -> String {
        self.data.extract_url()
    }
}

#[allow(unused)]
mod tests {
    use std::sync::atomic::AtomicI32;
    use std::sync::atomic::Ordering::SeqCst;

    use super::{API_RANGE, get_random_image};

    pub(crate) static API_CHOICE: AtomicI32 = AtomicI32::new(0);

    #[tokio::test]
    async fn test_random_image() {
        for i in API_RANGE {
            API_CHOICE.store(i, SeqCst);
            get_random_image("", "")
                .await
                .unwrap_or_else(|_| panic!("Failed to get random image from API {}", i));
        }
    }
}
