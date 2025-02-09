pub mod db;
pub mod models;
use crate::models::{PostScoring, TextInPost};
use log::info;
use once_cell::sync::Lazy;
use regex::Regex;
use rustrict::CensorStr;

//** NOTICE **
// This Bluesky feed intent is to highlight and uplift tech discussions and projects that are done via a thread or blog post and usually includes
// tutorials or other educational/deep dive materials with insights to a more indepth discussion about a subject along those lines
//

//TODO may do a regex of common words like computer, embedded, etc. Then a more in depth check?

static PROGRAMMER_JARGON: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(Rust|C\+\+|cpp|js|c#|swift|dotnet|php|Python|JavaScript|RustLang|Embedded dev|Microcontroller|IoT|Arduino|RaspberryPi|Programming|Software Developer|Software Developers|Dev|Hardware|Compiler|OpenSource|GitHub|Linux|Kernel|RTOS|ESP32|Pico|rp\s?2040|rp\s?2350|Micropython|VS Code|JetBrains|spi|i2c|soldering|waveshare|maker|adafruit)\b")
        .unwrap()
});

static BLOG_JARGON: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\b(blog|post|article|thread|write-up|guide|tutorial|how-to|explainer|deep dive|🧵|working|threads|project)\b",
    )
    .unwrap()
});

static DO_NOT_POST: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\b(musk|elon|trump|united states|flordia|texas|doge|government|president|potus|maga|vance)\b",
    )
    .unwrap()
});

pub fn does_the_post_belong_to_the_feed(all_text_in_post: Vec<TextInPost>) -> Option<PostScoring> {
    let mut contains_identifier_its_a_blog_or_thread = false;
    let mut should_be_saved = false;
    //Is it programming?
    let mut fits_topic = false;
    let mut post_text = String::new();
    let mut scoring = 0;
    for text in all_text_in_post {
        let string_of_text = text.clone().to_string();
        let should_it_be_censored = string_of_text.is_inappropriate();
        if should_it_be_censored {
            //Turns out it's a lot lol
            if PROGRAMMER_JARGON.is_match(string_of_text.as_str()) {
                info!("False positive to check?: {string_of_text}");
            }
            return None;
        }

        if DO_NOT_POST.is_match(string_of_text.as_str()) {
            return None;
        }

        //TODO check if it has links or like if it found the tech stuff in the link to post, or if in the post and theres replies?
        match text {
            TextInPost::Post(post) => {
                post_text = post.clone();
                if PROGRAMMER_JARGON.is_match(post.as_str()) {
                    scoring += 10;
                    fits_topic = true;
                }
                if BLOG_JARGON.is_match(post.as_str()) {
                    scoring += 30;
                    contains_identifier_its_a_blog_or_thread = true;
                    should_be_saved = true;
                }
            }
            TextInPost::Picture(picture) => {
                if PROGRAMMER_JARGON.is_match(picture.as_str()) {
                    scoring += 15;
                    fits_topic = true;
                }
                if BLOG_JARGON.is_match(picture.as_str()) {
                    scoring += 30;
                    contains_identifier_its_a_blog_or_thread = true;
                    should_be_saved = true;
                }
            }
            TextInPost::Video(video) => {
                if PROGRAMMER_JARGON.is_match(video.as_str()) {
                    scoring += 15;
                    fits_topic = true;
                }
                if BLOG_JARGON.is_match(video.as_str()) {
                    scoring += 15;
                    contains_identifier_its_a_blog_or_thread = true;
                    should_be_saved = true;
                }
            }
            TextInPost::External(external) => {
                if PROGRAMMER_JARGON.is_match(external.as_str()) {
                    scoring += 15;
                    fits_topic = true;
                }
                if BLOG_JARGON.is_match(external.as_str()) {
                    scoring += 30;
                    contains_identifier_its_a_blog_or_thread = true;
                    should_be_saved = true;
                }
            }
        };

        //TODO later may check if its blog or thread and only save then
        //should_be_saved
        if fits_topic && should_be_saved {
            return Some(PostScoring {
                pinned: false,
                deleted: false,
                priority: scoring,
            });
        }
    }

    None
}

mod tests {
    use crate::does_the_post_belong_to_the_feed;
    use crate::models::{PostScoring, TextInPost};

    #[test]
    fn test_post_scoring() {
        let post = "Welcome to the rust blog programming language blog!";
        let score = does_the_post_belong_to_the_feed(vec![TextInPost::Post(post.to_string())]);
        assert_eq!(
            score,
            Some(PostScoring {
                pinned: false,
                deleted: false,
                //Lower scoring because no pictures or links
                priority: 40
            })
        );
        print!("{:?}", score);
    }
}
