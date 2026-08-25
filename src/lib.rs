//! `duelchannel.ringrace.rs` backend.
//!
//! This provides a backend for the database system of the Duel Channel.

#![feature(never_type)]

pub mod app;
pub mod auth;
pub mod body;
pub mod cli;
pub mod config;
pub mod error;
pub mod multipart;
pub mod routes;
pub mod entity;
pub mod session;
pub mod short_id;
pub mod validate;
