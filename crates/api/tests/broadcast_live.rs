//! Going live and ending a broadcast end to end: the real router, real
//! token signing, a fresh Postgres database per test, and a counting
//! stand-in for MediaMTX's kick.

mod support;

use std::sync::{Arc, atomic::Ordering};

use axum::{
    Router,
    http::{Method, StatusCode, header},
};
use base64ct::{Base64UrlUnpadded, Encoding};
use ed25519_dalek::{Signature, VerifyingKey};
use infrastructure::media::jwt::PUBLISH_TOKEN_TTL;
use serde_json::{Value, json};
use sqlx::PgPool;
use support::{CountingTerminator, PASSWORD, Reply, live_state, request, send, sign_up};
use tokio::task::JoinSet;

struct World {
    pool: PgPool,
    router: Router,
    kicks: Arc<CountingTerminator>,
    owner: String,
    other: String,
}

impl World {
    async fn new(pool: PgPool) -> Self {
        let (state, kicks) = live_state(&pool, false).await;
        let router = api::app(state);
        let owner = sign_up(&router, "owner", PASSWORD).await.token();
        let other = sign_up(&router, "other", PASSWORD).await.token();
        Self {
            pool,
            router,
            kicks,
            owner,
            other,
        }
    }

    fn kicks(&self) -> usize {
        self.kicks.kicks.load(Ordering::SeqCst)
    }

    /// A fresh broadcast of the owner's, forced into `status`.
    async fn broadcast_in(&self, status: &str) -> String {
        let reply = send(
            &self.router,
            request(
                Method::POST,
                "/api/broadcasts",
                Some(json!({"title": "라이브", "description": ""})),
                Some(&self.owner),
            ),
        )
        .await;
        assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
        let id = reply.body["id"].as_str().unwrap().to_owned();
        sqlx::query("UPDATE broadcasts SET status = $2 WHERE id = $1")
            .bind(id.parse::<i64>().unwrap())
            .bind(status)
            .execute(&self.pool)
            .await
            .unwrap();
        id
    }

    async fn stored(&self, id: &str) -> Stored {
        let (status, version, started, ended) = sqlx::query_as::<_, (String, i32, bool, bool)>(
            "SELECT status, version, started_at IS NOT NULL, ended_at IS NOT NULL \
             FROM broadcasts WHERE id = $1",
        )
        .bind(id.parse::<i64>().unwrap())
        .fetch_one(&self.pool)
        .await
        .unwrap();
        Stored {
            status,
            version,
            started,
            ended,
        }
    }
}

#[derive(Debug, PartialEq)]
struct Stored {
    status: String,
    version: i32,
    started: bool,
    ended: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    Start,
    Stop,
}

impl Op {
    fn path(self, id: &str) -> String {
        match self {
            Self::Start => format!("/api/broadcasts/{id}/start"),
            Self::Stop => format!("/api/broadcasts/{id}/stop"),
        }
    }
}

async fn call(router: &Router, op: Op, id: &str, token: Option<&str>) -> Reply {
    send(router, request(Method::POST, &op.path(id), None, token)).await
}

#[derive(Debug, Clone, Copy)]
enum Actor {
    Anonymous,
    Owner,
    Other,
}

/// `(op, actor, status before, expected status code, error code, status after)`.
type Row = (
    Op,
    Actor,
    &'static str,
    StatusCode,
    Option<&'static str>,
    &'static str,
);

/// The owner's start on a `LIVE` broadcast reissues a token and changes nothing.
const MATRIX: [Row; 24] = {
    use Actor::*;
    use Op::*;
    const OK: StatusCode = StatusCode::OK;
    const UNAUTH: StatusCode = StatusCode::UNAUTHORIZED;
    const FORBID: StatusCode = StatusCode::FORBIDDEN;
    const CONFLICT: StatusCode = StatusCode::CONFLICT;
    const ANON: Option<&str> = Some("UNAUTHENTICATED");
    const NOT_OWNER: Option<&str> = Some("NOT_BROADCAST_OWNER");
    const NOT_STARTABLE: Option<&str> = Some("BROADCAST_NOT_STARTABLE");
    const NOT_LIVE: Option<&str> = Some("BROADCAST_NOT_LIVE");
    [
        (Start, Anonymous, "READY", UNAUTH, ANON, "READY"),
        (Start, Anonymous, "LIVE", UNAUTH, ANON, "LIVE"),
        (Start, Anonymous, "INTERRUPTED", UNAUTH, ANON, "INTERRUPTED"),
        (Start, Anonymous, "ENDED", UNAUTH, ANON, "ENDED"),
        (Start, Other, "READY", FORBID, NOT_OWNER, "READY"),
        (Start, Other, "LIVE", FORBID, NOT_OWNER, "LIVE"),
        (
            Start,
            Other,
            "INTERRUPTED",
            FORBID,
            NOT_OWNER,
            "INTERRUPTED",
        ),
        (Start, Other, "ENDED", FORBID, NOT_OWNER, "ENDED"),
        (Start, Owner, "READY", OK, None, "LIVE"),
        (Start, Owner, "LIVE", OK, None, "LIVE"),
        (
            Start,
            Owner,
            "INTERRUPTED",
            CONFLICT,
            NOT_STARTABLE,
            "INTERRUPTED",
        ),
        (Start, Owner, "ENDED", CONFLICT, NOT_STARTABLE, "ENDED"),
        (Stop, Anonymous, "READY", UNAUTH, ANON, "READY"),
        (Stop, Anonymous, "LIVE", UNAUTH, ANON, "LIVE"),
        (Stop, Anonymous, "INTERRUPTED", UNAUTH, ANON, "INTERRUPTED"),
        (Stop, Anonymous, "ENDED", UNAUTH, ANON, "ENDED"),
        (Stop, Other, "READY", FORBID, NOT_OWNER, "READY"),
        (Stop, Other, "LIVE", FORBID, NOT_OWNER, "LIVE"),
        (Stop, Other, "INTERRUPTED", FORBID, NOT_OWNER, "INTERRUPTED"),
        (Stop, Other, "ENDED", FORBID, NOT_OWNER, "ENDED"),
        (Stop, Owner, "READY", CONFLICT, NOT_LIVE, "READY"),
        (Stop, Owner, "LIVE", OK, None, "ENDED"),
        (Stop, Owner, "INTERRUPTED", OK, None, "ENDED"),
        (Stop, Owner, "ENDED", CONFLICT, NOT_LIVE, "ENDED"),
    ]
};

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn authorization_and_status_matrix(pool: PgPool) {
    let world = World::new(pool).await;

    for (op, actor, before, expected, code, after) in MATRIX {
        let row = format!("{op:?} as {actor:?} on {before}");
        let token = match actor {
            Actor::Anonymous => None,
            Actor::Owner => Some(world.owner.as_str()),
            Actor::Other => Some(world.other.as_str()),
        };
        let id = world.broadcast_in(before).await;
        let stored_before = world.stored(&id).await;
        let kicks_before = world.kicks();

        let reply = call(&world.router, op, &id, token).await;

        assert_eq!(reply.status, expected, "{row}: {}", reply.body);
        assert_eq!(reply.code(), code.unwrap_or_default(), "{row}");
        let stored = world.stored(&id).await;
        assert_eq!(stored.status, after, "{row}");
        let transitioned = before != after;
        let version = stored_before.version + i32::from(transitioned);
        assert_eq!(
            stored.version, version,
            "{row}: every transition bumps the version"
        );
        if !transitioned {
            assert_eq!(stored, stored_before, "{row} changed the row");
        }
        let kicked = op == Op::Stop && transitioned;
        assert_eq!(world.kicks() - kicks_before, usize::from(kicked), "{row}");

        match (op, reply.status) {
            (Op::Start, StatusCode::OK) => {
                assert_eq!(reply.headers[header::CACHE_CONTROL], "no-store", "{row}");
                assert_eq!(
                    reply.body["whipUrl"],
                    format!("/media/webrtc/live/{id}/whip"),
                    "{row}"
                );
                assert!(stored.started || before == "LIVE", "{row}");
            }
            (Op::Stop, StatusCode::OK) => {
                assert_eq!(reply.body["status"], "ENDED", "{row}");
                assert_eq!(reply.body["version"], stored.version, "{row}");
                assert_eq!(
                    reply.headers[header::ETAG],
                    format!("\"{}\"", stored.version),
                    "{row}"
                );
                assert!(stored.ended, "{row}");
            }
            _ => {}
        }
    }
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn missing_and_deleted_broadcasts_are_not_found(pool: PgPool) {
    let world = World::new(pool).await;
    let deleted = world.broadcast_in("READY").await;
    let reply = send(
        &world.router,
        request(
            Method::DELETE,
            &format!("/api/broadcasts/{deleted}"),
            None,
            Some(&world.owner),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);

    for op in [Op::Start, Op::Stop] {
        for id in [deleted.as_str(), "123", "abc"] {
            let reply = call(&world.router, op, id, Some(&world.owner)).await;
            assert_eq!(reply.status, StatusCode::NOT_FOUND, "{op:?} {id}");
            assert_eq!(reply.code(), "BROADCAST_NOT_FOUND", "{op:?} {id}");
        }
    }
    assert_eq!(world.kicks(), 0);
}

fn decode(segment: &str) -> Vec<u8> {
    Base64UrlUnpadded::decode_vec(segment).unwrap()
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn the_publish_token_verifies_against_the_served_jwks(pool: PgPool) {
    let world = World::new(pool).await;
    let jwks = send(
        &world.router,
        request(Method::GET, "/api/.well-known/jwks.json", None, None),
    )
    .await;
    assert_eq!(jwks.status, StatusCode::OK);
    assert_eq!(jwks.headers[header::CACHE_CONTROL], "public, max-age=300");
    let keys = jwks.body["keys"].as_array().unwrap();
    assert_eq!(keys.len(), 1);
    let jwk = &keys[0];
    for (name, value) in [
        ("kty", "OKP"),
        ("crv", "Ed25519"),
        ("alg", "EdDSA"),
        ("use", "sig"),
    ] {
        assert_eq!(jwk[name], value, "{name}");
    }
    let x: [u8; 32] = decode(jwk["x"].as_str().unwrap()).try_into().unwrap();
    let public = VerifyingKey::from_bytes(&x).unwrap();

    let id = world.broadcast_in("READY").await;
    let mut jtis = Vec::new();
    for _ in 0..2 {
        let reply = call(&world.router, Op::Start, &id, Some(&world.owner)).await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        assert_eq!(reply.body["expiresIn"], PUBLISH_TOKEN_TTL.as_secs());
        let token = reply.body["token"].as_str().unwrap();
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3);
        let header: Value = serde_json::from_slice(&decode(parts[0])).unwrap();
        let claims: Value = serde_json::from_slice(&decode(parts[1])).unwrap();
        let signature = Signature::from_slice(&decode(parts[2])).unwrap();

        public
            .verify_strict(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
            .expect("signed by the JWKS key");
        assert_eq!(header["alg"], "EdDSA");
        assert_eq!(header["kid"], jwk["kid"]);
        let lifetime = claims["exp"].as_u64().unwrap() - claims["iat"].as_u64().unwrap();
        assert_eq!(lifetime, PUBLISH_TOKEN_TTL.as_secs());
        assert_eq!(
            claims["mediamtx_permissions"],
            json!([{"action": "publish", "path": format!("live/{id}")}])
        );
        jtis.push(claims["jti"].as_str().unwrap().to_owned());
    }
    assert_ne!(jtis[0], jtis[1], "every token has its own jti");
}

/// Sends `ops` against one broadcast all at once, each from its own task.
async fn concurrently(
    world: &World,
    id: &str,
    ops: impl IntoIterator<Item = Op>,
) -> Vec<(Op, Reply)> {
    let mut tasks = JoinSet::new();
    for op in ops {
        let router = world.router.clone();
        let (id, token) = (id.to_owned(), world.owner.clone());
        tasks.spawn(async move { (op, call(&router, op, &id, Some(&token)).await) });
    }
    tasks.join_all().await
}

const ROUNDS: usize = 10;

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn fifty_concurrent_starts_go_live_once(pool: PgPool) {
    let world = World::new(pool).await;
    for round in 0..ROUNDS {
        let id = world.broadcast_in("READY").await;
        let replies = concurrently(&world, &id, [Op::Start; 50]).await;

        for (_, reply) in &replies {
            assert_eq!(
                reply.status,
                StatusCode::OK,
                "round {round}: {}",
                reply.body
            );
        }
        let stored = world.stored(&id).await;
        assert_eq!(
            (stored.status.as_str(), stored.version),
            ("LIVE", 2),
            "round {round}"
        );
    }
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn fifty_concurrent_stops_end_and_kick_once(pool: PgPool) {
    let world = World::new(pool).await;
    for round in 0..ROUNDS {
        let id = world.broadcast_in("LIVE").await;
        let kicks_before = world.kicks();
        let replies = concurrently(&world, &id, [Op::Stop; 50]).await;

        let ok = replies
            .iter()
            .filter(|(_, r)| r.status == StatusCode::OK)
            .count();
        assert_eq!(ok, 1, "round {round}");
        for (_, reply) in replies.iter().filter(|(_, r)| r.status != StatusCode::OK) {
            assert_eq!(reply.status, StatusCode::CONFLICT, "round {round}");
            assert_eq!(reply.code(), "BROADCAST_NOT_LIVE", "round {round}");
        }
        assert_eq!(world.kicks() - kicks_before, 1, "round {round}");
        let stored = world.stored(&id).await;
        assert_eq!(
            (stored.status.as_str(), stored.version),
            ("ENDED", 2),
            "round {round}"
        );
    }
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn mixed_starts_and_stops_transition_at_most_once_each(pool: PgPool) {
    let world = World::new(pool).await;
    for round in 0..ROUNDS {
        let id = world.broadcast_in("READY").await;
        let kicks_before = world.kicks();
        let ops = (0..50).map(|i| if i % 2 == 0 { Op::Start } else { Op::Stop });
        let replies = concurrently(&world, &id, ops).await;

        let count = |op: Op, status: StatusCode| {
            replies
                .iter()
                .filter(|(o, r)| *o == op && r.status == status)
                .count()
        };
        let (starts_ok, stops_ok) = (
            count(Op::Start, StatusCode::OK),
            count(Op::Stop, StatusCode::OK),
        );
        assert_eq!(
            starts_ok + count(Op::Start, StatusCode::CONFLICT),
            25,
            "round {round}"
        );
        assert_eq!(
            stops_ok + count(Op::Stop, StatusCode::CONFLICT),
            25,
            "round {round}"
        );
        assert!(stops_ok <= 1, "round {round}: {stops_ok} stops ended it");
        assert_eq!(world.kicks() - kicks_before, stops_ok, "round {round}");

        // A stop can only succeed after a start took the broadcast live, so
        // the final row follows from whether a stop succeeded.
        let stored = world.stored(&id).await;
        let expected = match (starts_ok, stops_ok) {
            (0, 0) => ("READY", 1),
            (_, 0) => ("LIVE", 2),
            (1.., 1) => ("ENDED", 3),
            other => panic!("round {round}: impossible outcome {other:?}"),
        };
        assert_eq!(
            (stored.status.as_str(), stored.version),
            expected,
            "round {round}"
        );
    }
}
