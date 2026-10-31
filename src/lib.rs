//! KubeShelf: a Kubernetes-aware app shelf (Rust port of the original Go app).
//!
//! Module map (Go file → Rust module):
//! * `model.go`, `favorites.go` (types/validation) → [`model`]
//! * `net/url` behaviour the Go code relies on      → [`gourl`]
//! * `ConfigFromEnv` in `model.go`                  → [`config`]
//! * `catalog.go`                                   → [`catalog`]
//! * Git parts of `store.go`/`favorite_store.go`    → [`git`]
//! * the `Store` type in those files                → [`store`]
//! * `auth.go`                                      → [`auth`]
//! * `server.go`, `routes.go`, HTTP parts of `favorites.go` → [`http`]
//! * `source.go`, `user_settings_watch.go`          → [`source`]
//! * `cmd/kubeshelf/main.go`                        → `src/main.rs`
pub mod auth;
pub mod catalog;
pub mod config;
pub mod git;
pub mod gourl;
pub mod http;
pub mod model;
pub mod source;
pub mod store;
