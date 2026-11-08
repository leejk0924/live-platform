//! Adapters that implement `application` ports against concrete infrastructure.

pub mod auth;
pub mod id;
pub mod media;
pub mod payment;
pub mod persistence;
pub mod rate_limit;
pub mod realtime;
pub mod recording;
