use atom_syndication;
use base64::{decode, encode};
use chrono::{DateTime, Utc};
use rss;
use sha2::{Digest, Sha256};
use std::str;
use warp::ws::Message;

use db::get_user;
use schema::*;
use web::types::IncomingMessageType;

//////////
// Feed //
//////////

#[derive(Debug, Queryable, Associations, Identifiable, Serialize)]
pub struct Feed {
  pub id: i32,
  pub title: String,
  pub description: Option<String>,
  pub site_link: String,
  pub feed_link: String,
  pub updated_at: DateTime<Utc>,
}

#[derive(Insertable)]
#[table_name = "feeds"]
pub struct NewFeed {
  pub title: String,
  pub description: Option<String>,
  pub site_link: String,
  pub feed_link: String,
  pub updated_at: DateTime<Utc>,
}
impl NewFeed {
  pub fn from_rss(feed: &rss::Channel, url: &str) -> NewFeed {
    NewFeed {
      title: feed.title().to_string(),
      site_link: feed.link().to_string(),
      feed_link: url.to_string(),
      description: Some(feed.description().to_string()),
      updated_at: Utc::now(),
    }
  }

  pub fn from_atom(feed: &atom_syndication::Feed, url: &str) -> NewFeed {
    NewFeed {
      title: feed.title().to_string(),
      site_link: feed.links()[0].href().to_string(),
      feed_link: url.to_string(),
      description: feed.subtitle().and_then(|s| Some(s.to_owned())),
      updated_at: Utc::now(),
    }
  }
}

//////////
// Item //
//////////

#[derive(Debug, Queryable, Associations, Identifiable, Serialize)]
#[belongs_to(Feed)]
pub struct Item {
  pub id: i32,
  #[serde(skip_serializing)]
  pub guid: String,
  pub link: String,
  pub title: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub summary: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub content: Option<String>,
  pub published_at: Option<DateTime<Utc>>,
  pub updated_at: Option<DateTime<Utc>>,
  #[serde(skip_serializing)]
  pub feed_id: i32,
}

#[derive(Insertable, AsChangeset, Debug)]
#[table_name = "items"]
pub struct NewItem {
  pub guid: String,
  pub link: String,
  pub title: String,
  pub summary: Option<String>,
  pub content: Option<String>,
  pub published_at: Option<DateTime<Utc>>,
  pub updated_at: Option<DateTime<Utc>>,
  pub feed_id: i32,
}
impl NewItem {
  pub fn from_item(item: &rss::Item, feed_id: i32) -> NewItem {
    NewItem {
      guid: item.guid().unwrap().value().to_owned(),
      title: item.title().expect("no title!").to_owned(),
      link: item.link().expect("no link!").to_owned(),
      summary: item.description().and_then(|s| Some(s.to_owned())),
      content: item.content().and_then(|s| Some(s.to_owned())),
      published_at: item.pub_date().and_then(|d| parse_date(d)),
      updated_at: item.pub_date().and_then(|d| parse_date(d)),
      feed_id: feed_id,
    }
  }
  pub fn from_entry(item: &atom_syndication::Entry, feed_id: i32) -> NewItem {
    NewItem {
      guid: item.id().to_owned(),
      title: item.title().to_owned(),
      link: item.links()[0].href().to_owned(),
      summary: item.summary().and_then(|s| Some(s.to_owned())),
      content: item
        .content()
        .and_then(|o| o.value().and_then(|s| Some(s.to_owned()))),
      published_at: item.published().and_then(|d| parse_date(d)),
      updated_at: parse_date(item.updated()),
      feed_id: feed_id,
    }
  }
}

//////////////////
// Subscription //
//////////////////

#[derive(Debug, Queryable, Serialize)]
pub struct SubscribedItem {
  pub id: i32,
  #[serde(skip_serializing)]
  pub guid: String,
  pub link: String,
  pub title: String,
  pub summary: Option<String>,
  pub content: Option<String>,
  pub published_at: Option<DateTime<Utc>>,
  pub updated_at: Option<DateTime<Utc>>,
  pub feed_id: i32,
  pub subscribed_item_id: i32,
  pub user_id: i32,
  pub seen: bool,
}

#[derive(Debug, Queryable, Serialize, Associations)]
#[belongs_to(User)]
pub struct SubscribedFeed {
  pub id: i32,
  pub title: String,
  pub description: Option<String>,
  pub site_link: String,
  pub feed_link: String,
  pub updated_at: DateTime<Utc>,
  pub user_id: i32,
  pub unseen_count: i32,
}

///////////////
// Composite //
///////////////

#[derive(Debug, Serialize, Clone)]
pub struct CompositeItem {
  pub id: i32,
  pub title: String,
  pub link: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub summary: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub content: Option<String>,
  pub published_at: Option<DateTime<Utc>>,
  pub updated_at: Option<DateTime<Utc>>,
  pub seen: bool,
}
impl CompositeItem {
  pub fn from_item(item: &Item) -> Self {
    CompositeItem {
      id: item.id,
      title: item.title.clone(),
      link: item.link.clone(),
      summary: item.summary.clone(),
      content: item.content.clone(),
      published_at: item.published_at,
      updated_at: item.updated_at,
      seen: false,
    }
  }
  pub fn from_subscribed(item: &SubscribedItem) -> Self {
    CompositeItem {
      id: item.id,
      title: item.title.clone(),
      link: item.link.clone(),
      summary: item.summary.clone(),
      content: item.content.clone(),
      published_at: item.published_at,
      updated_at: item.updated_at,
      seen: item.seen,
    }
  }
}

//////////
// User //
//////////

#[derive(Debug, Queryable, Associations, Identifiable, Serialize)]
pub struct User {
  pub id: i32,
  pub username: String,
  pub password_hash: Vec<u8>,
}
impl User {
  pub fn check_user(username: &str, pass: &str) -> Option<User> {
    match get_user(username) {
      Some(user) => match user.verifies(pass) {
        true => Some(user),
        false => None,
      },
      None => None,
    }
  }

  pub fn hash_pw(s: &str) -> String {
    let mut hasher = Sha256::default();
    hasher.input(s.as_bytes());
    let output = hasher.result();
    let hash = &output[..];
    let e = encode(hash);
    e
  }

  fn verifies(&self, pass: &str) -> bool {
    let orig_hash = decode(&self.password_hash).unwrap();
    let mut hasher = Sha256::default();
    hasher.input(pass.as_bytes());
    let output = hasher.result();
    let hashed_pw = &output[..];
    orig_hash == hashed_pw
  }
}

////////////
// Claims //
////////////

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
  pub name: String,
  pub id: i32,
}

fn parse_date(date: &str) -> Option<DateTime<Utc>> {
  match DateTime::parse_from_rfc2822(date) {
    Ok(d) => Some(d.with_timezone(&Utc)),
    Err(_) => date.parse::<DateTime<Utc>>().ok(),
  }
}

///////////////
// Websocket //
///////////////

#[derive(Debug, Serialize)]
pub enum OutgoingWebsocketMessageType {
  NewFeed,
  NewItems,
  ActionResult,
}
#[derive(Debug, Serialize)]
pub enum OutgoingWebsocketMessageData {
  NewFeed(FeedMessage),
  NewItems(ItemsMessage),
  ActionResult(ResultMessage),
}
#[derive(Debug, Serialize)]
pub struct OutgoingWebsocketMessage {
  id: OutgoingWebsocketMessageType,
  pub data: OutgoingWebsocketMessageData,
}
impl OutgoingWebsocketMessage {
  pub fn new_feed(feed: SubscribedFeed) -> Self {
    let p = FeedMessage {
      feed_id: feed.id,
      feed: feed,
    };
    OutgoingWebsocketMessage {
      id: OutgoingWebsocketMessageType::NewFeed,
      data: OutgoingWebsocketMessageData::NewFeed(p),
    }
  }
  pub fn new_items(feed_id: i32, items: Vec<CompositeItem>) -> Self {
    let p = ItemsMessage {
      feed_id: feed_id,
      items: items,
    };
    OutgoingWebsocketMessage {
      id: OutgoingWebsocketMessageType::NewItems,
      data: OutgoingWebsocketMessageData::NewItems(p),
    }
  }
  pub fn action_result(action: IncomingMessageType, result: bool) -> Self {
    let p = ResultMessage {
      id: action,
      result: result,
    };
    OutgoingWebsocketMessage {
      id: OutgoingWebsocketMessageType::ActionResult,
      data: OutgoingWebsocketMessageData::ActionResult(p),
    }
  }
  pub fn to_message(&self) -> Message {
    let msg = json!(self);
    Message::text(msg.to_string())
  }
}

#[derive(Debug, Serialize)]
pub struct FeedMessage {
  pub feed_id: i32,
  pub feed: SubscribedFeed,
}
#[derive(Debug, Serialize)]
pub struct ItemsMessage {
  pub feed_id: i32,
  pub items: Vec<CompositeItem>,
}
#[derive(Serialize, Debug)]
pub struct ResultMessage {
  pub id: IncomingMessageType,
  pub result: bool,
}
