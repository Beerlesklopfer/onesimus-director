//! DirectorApi: configuration resources as raw text (CRUD; Onesimus
//! extension, docs/plans/CONFIG-CRUD-GRPC.md, logic in `config_edit`).
//!
//! Rights (principles 7 and 13):
//! - every RPC needs a session (UNAUTHENTICATED otherwise);
//! - Read/Create/Update/Delete/List need the Command ACL `configure`
//!   (gRPC PERMISSION_DENIED otherwise, like the other DirectorApi RPCs):
//!   the raw text is not redacted like `show` without `configure`;
//! - the resource ACL of the type (Schedule ACL, Job ACL for Job and
//!   JobDefs, Client ACL, as `show`) decides per name: status DENIED; ListResources
//!   leaves out what it denies;
//! - GetConfigCapabilities never fails for a session: without `configure`
//!   it lists no type, otherwise the types of this stage whose ACL allows
//!   any resource, with the operations implemented (RENAME later).
//!
//! These RPCs exist in every build (unlike the S-35 write RPCs): they have
//! their own safeguards (optimistic lock, `-t` check, rollback).

#![allow(clippy::result_large_err)] // tonic::Status, as everywhere in the gRPC layer

use std::sync::Arc;

use tonic::{Request, Response, Status};

use onesimus_proto_rs::onesimus::director::api as pb;

use super::api::DirectorApiImpl;
use crate::config_edit::{self, ConfigEditor, Outcome, RType};

/// interface_version of GetConfigCapabilities.
const INTERFACE_VERSION: u32 = 1;

/// Operations implemented for every type of this stage.
const OPERATIONS: [pb::ConfigOperation; 4] = [
    pb::ConfigOperation::ReadRaw,
    pb::ConfigOperation::CreateRaw,
    pb::ConfigOperation::UpdateRaw,
    pb::ConfigOperation::Delete,
];

fn to_proto_type(rtype: RType) -> pb::ResourceType {
    match rtype {
        RType::Schedule => pb::ResourceType::Schedule,
        RType::Job => pb::ResourceType::Job,
        RType::JobDefs => pb::ResourceType::Jobdefs,
        RType::Client => pb::ResourceType::Client,
        RType::FileSet => pb::ResourceType::Fileset,
        RType::Pool => pb::ResourceType::Pool,
        RType::Storage => pb::ResourceType::Storage,
    }
}

/// Proto type of a normalized raw resource type (referenced_by).
fn proto_type_of_norm(norm: &str) -> pb::ResourceType {
    match norm {
        "schedule" => pb::ResourceType::Schedule,
        "job" => pb::ResourceType::Job,
        "jobdefs" => pb::ResourceType::Jobdefs,
        "client" => pb::ResourceType::Client,
        "fileset" => pb::ResourceType::Fileset,
        "pool" => pb::ResourceType::Pool,
        "storage" => pb::ResourceType::Storage,
        "messages" => pb::ResourceType::Messages,
        "catalog" => pb::ResourceType::Catalog,
        "console" => pb::ResourceType::Console,
        "profile" => pb::ResourceType::Profile,
        "counter" => pb::ResourceType::Counter,
        "user" => pb::ResourceType::User,
        "director" => pb::ResourceType::Director,
        _ => pb::ResourceType::Unspecified,
    }
}

/// The edited type of a request: INVALID_ARGUMENT without a type,
/// UNIMPLEMENTED for types of later stages.
fn rtype(value: i32) -> Result<RType, Status> {
    let t = pb::ResourceType::try_from(value).unwrap_or(pb::ResourceType::Unspecified);
    match t {
        pb::ResourceType::Unspecified => Err(Status::invalid_argument("resource type is required")),
        pb::ResourceType::Schedule => Ok(RType::Schedule),
        pb::ResourceType::Job => Ok(RType::Job),
        pb::ResourceType::Jobdefs => Ok(RType::JobDefs),
        pb::ResourceType::Client => Ok(RType::Client),
        pb::ResourceType::Fileset => Ok(RType::FileSet),
        pb::ResourceType::Pool => Ok(RType::Pool),
        pb::ResourceType::Storage => Ok(RType::Storage),
        other => Err(Status::unimplemented(format!("{} cannot be edited yet (see GetConfigCapabilities)", other.as_str_name()))),
    }
}

fn resource_ref(resource: Option<pb::ResourceRef>) -> Result<(RType, String), Status> {
    let resource = resource.ok_or_else(|| Status::invalid_argument("resource is required"))?;
    let rtype = rtype(resource.r#type)?;
    if resource.name.is_empty() {
        return Err(Status::invalid_argument("resource.name is required"));
    }
    Ok((rtype, resource.name))
}

fn response(rtype: RType, o: Outcome) -> pb::ResourceResponse {
    let status = match o.status {
        config_edit::Status::Ok => pb::ConfigStatus::Ok,
        config_edit::Status::Conflict => pb::ConfigStatus::Conflict,
        config_edit::Status::Invalid => pb::ConfigStatus::Invalid,
        config_edit::Status::Referenced => pb::ConfigStatus::Referenced,
        config_edit::Status::Denied => pb::ConfigStatus::Denied,
        config_edit::Status::NotFound => pb::ConfigStatus::NotFound,
        config_edit::Status::Exists => pb::ConfigStatus::Exists,
        config_edit::Status::ReloadFailed => pb::ConfigStatus::ReloadFailed,
    };
    pb::ResourceResponse {
        status: status as i32,
        message: o.message,
        resource: Some(pb::ResourceRef { r#type: to_proto_type(rtype) as i32, name: o.name }),
        file: o.file,
        text: o.text,
        sha256: o.sha256,
        shared_file: o.shared_file,
        referenced_by: o.referenced_by.into_iter()
            .map(|(t, name)| pb::ResourceRef { r#type: proto_type_of_norm(&t) as i32, name }).collect(),
        problem: o.problem.map(|p| pb::ConfigProblem { file: p.file, line: p.line, column: p.column, text: p.text }),
        reloaded: o.reloaded,
        validated_only: o.validated_only,
        old_sha256: o.old_sha256,
        followups: o.followups,
    }
}

/// Session of a CRUD call: (console, session id, ACL decision per name).
struct Caller {
    console: String,
    allow: Arc<dyn Fn(&str) -> bool + Send + Sync>,
}

impl DirectorApiImpl {
    fn config_editor(&self) -> Result<Arc<ConfigEditor>, Status> {
        if let Some(editor) = self.config_editor.get() {
            return Ok(editor.clone());
        }
        // the editor of the `.config` console command (one lock for both paths)
        self.exec.as_ref().and_then(|exec| config_edit::shared_editor(&exec.config))
            .ok_or_else(|| Status::failed_precondition("configuration editing is not available (no configuration path)"))
    }

    /// Session check, Command ACL `configure`, the resource ACL of `rtype`.
    fn caller<Q>(&self, request: &Request<Q>, rtype: RType) -> Result<Caller, Status> {
        let (console, id) = self.auth.identity(request.metadata())?;
        if !self.auth.allowed(id, crate::acl::Kind::Command, "configure") {
            return Err(Status::permission_denied("command \"configure\" not allowed"));
        }
        let auth = self.auth.clone();
        let kind = rtype.acl();
        Ok(Caller { console, allow: Arc::new(move |name: &str| auth.allowed(id, kind, name)) })
    }

    async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, Status> {
        tokio::task::spawn_blocking(f).await
            .map_err(|e| Status::internal(format!("configuration edit: {e}")))?
            .map_err(Status::internal)
    }

    pub(super) async fn config_capabilities(&self, request: Request<pb::ConfigCapabilitiesRequest>)
        -> Result<Response<pb::ConfigCapabilitiesResponse>, Status> {
        let (console, id) = self.auth.identity(request.metadata())?;
        let mut types = Vec::new();
        if self.auth.allowed(id, crate::acl::Kind::Command, "configure") {
            for rtype in RType::ALL {
                if self.auth.any_allowed(&console, rtype.acl()) {
                    types.push(pb::TypeCapabilities {
                        r#type: to_proto_type(rtype) as i32,
                        operations: OPERATIONS.iter().map(|o| *o as i32).collect(),
                    });
                }
            }
        }
        Ok(Response::new(pb::ConfigCapabilitiesResponse { interface_version: INTERFACE_VERSION, types }))
    }

    pub(super) async fn config_read(&self, request: Request<pb::ReadResourceRequest>)
        -> Result<Response<pb::ResourceResponse>, Status> {
        let editor = self.config_editor()?;
        let (rtype, name) = resource_ref(request.get_ref().resource.clone())?;
        let caller = self.caller(&request, rtype)?;
        let outcome = Self::blocking(move || editor.read(rtype, &name, &*caller.allow)).await?;
        Ok(Response::new(response(rtype, outcome)))
    }

    pub(super) async fn config_create(&self, request: Request<pb::CreateResourceRequest>)
        -> Result<Response<pb::ResourceResponse>, Status> {
        let editor = self.config_editor()?;
        let rtype = rtype(request.get_ref().r#type)?;
        let caller = self.caller(&request, rtype)?;
        let pb::CreateResourceRequest { text, validate_only, .. } = request.into_inner();
        let outcome = Self::blocking(move || editor.create(rtype, &text, validate_only, &caller.console, &*caller.allow)).await?;
        Ok(Response::new(response(rtype, outcome)))
    }

    pub(super) async fn config_update(&self, request: Request<pb::UpdateResourceRequest>)
        -> Result<Response<pb::ResourceResponse>, Status> {
        let editor = self.config_editor()?;
        let (rtype, name) = resource_ref(request.get_ref().resource.clone())?;
        let caller = self.caller(&request, rtype)?;
        let pb::UpdateResourceRequest { expected_sha256, text, validate_only, .. } = request.into_inner();
        let outcome = Self::blocking(move || editor.update(rtype, &name, &expected_sha256, &text, validate_only,
            &caller.console, &*caller.allow)).await?;
        Ok(Response::new(response(rtype, outcome)))
    }

    pub(super) async fn config_delete(&self, request: Request<pb::DeleteResourceRequest>)
        -> Result<Response<pb::ResourceResponse>, Status> {
        let editor = self.config_editor()?;
        let (rtype, name) = resource_ref(request.get_ref().resource.clone())?;
        let caller = self.caller(&request, rtype)?;
        let pb::DeleteResourceRequest { expected_sha256, validate_only, .. } = request.into_inner();
        let outcome = Self::blocking(move || editor.delete(rtype, &name, &expected_sha256, validate_only,
            &caller.console, &*caller.allow)).await?;
        Ok(Response::new(response(rtype, outcome)))
    }

    pub(super) async fn config_list(&self, request: Request<pb::ListResourcesRequest>)
        -> Result<Response<pb::ListResourcesResponse>, Status> {
        let editor = self.config_editor()?;
        let rtype = rtype(request.get_ref().r#type)?;
        let caller = self.caller(&request, rtype)?;
        let list = Self::blocking(move || editor.list(rtype, &*caller.allow)).await?;
        Ok(Response::new(pb::ListResourcesResponse {
            resources: list.into_iter().map(|s| pb::ResourceSummary {
                resource: Some(pb::ResourceRef { r#type: to_proto_type(rtype) as i32, name: s.name }),
                file: s.file,
                sha256: s.sha256,
                shared_file: s.shared_file,
            }).collect(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_edit::tests::{config_dir, WEEKLY};
    use crate::director_config::{DirectorConfig, SharedConfig};
    use onesimus_proto_rs::onesimus::director::api::director_api_server::DirectorApi;
    use std::sync::RwLock;

    /// DirectorApi with a throw-away configuration, consoles with different
    /// ACLs and a reload that installs the new configuration (no catalog).
    struct Fixture {
        api: DirectorApiImpl,
        auth: Arc<super::super::auth::AuthState>,
        dir: std::path::PathBuf,
        shared: SharedConfig,
    }

    fn fixture(name: &str) -> Fixture {
        let dir = config_dir(name);
        std::fs::write(dir.join("bareos-dir.d/console/consoles.conf"), "\
Console {\n  Name = admin\n  Password = pw\n  CommandACL = configure, show\n  ScheduleACL = *all*\n  JobACL = a, !secret\n}\n\
Console {\n  Name = viewer\n  Password = pw\n  CommandACL = show\n  ScheduleACL = *all*\n  JobACL = *all*\n}\n\
Console {\n  Name = noschedule\n  Password = pw\n  CommandACL = configure\n  ScheduleACL = !WeeklyCycle\n  JobACL = *all*\n}\n\
Console {\n  Name = clients\n  Password = pw\n  CommandACL = configure\n  ClientACL = fd\n}\n").unwrap();
        let shared: SharedConfig = Arc::new(RwLock::new(DirectorConfig::load(&dir).unwrap()));
        let manager = Arc::new(crate::session_manager::SessionManager::new(Arc::new(crate::message_bus::MessageBus::new(16))));
        let auth = Arc::new(super::super::auth::AuthState::new(shared.clone(), manager));
        let (d, s) = (dir.clone(), shared.clone());
        let editor = ConfigEditor::new(dir.clone(), Box::new(move || {
            let config = DirectorConfig::load(&d).map_err(|e| e.to_string())?;
            crate::config_check::check(&config.raw, &crate::config_check::base_config_path(&d))
                .map_err(|diags| diags[0].text().to_string())?;
            *s.write().unwrap() = config;
            Ok(())
        }), Box::new(|_| {}));
        let queue = Arc::new(crate::command_queue::CommandQueue::new(8));
        let api = DirectorApiImpl::new(queue, auth.clone()).with_config_editor(Arc::new(editor));
        Fixture { api, auth, dir, shared }
    }

    impl Fixture {
        fn login(&self, console: &str, password: &str) -> String {
            let nonce = self.auth.challenge(console);
            let proof = super::super::auth::compute_proof(&super::super::auth::password_key(password), &nonce);
            self.auth.login(console, &nonce, &proof).unwrap()
        }
    }

    fn req<Q>(message: Q, token: &str) -> Request<Q> {
        let mut request = Request::new(message);
        request.metadata_mut().insert(super::super::auth::SESSION_HEADER, token.parse().unwrap());
        request
    }

    fn schedule(name: &str) -> Option<pb::ResourceRef> {
        Some(pb::ResourceRef { r#type: pb::ResourceType::Schedule as i32, name: name.into() })
    }

    fn types(caps: &pb::ConfigCapabilitiesResponse) -> Vec<pb::ResourceType> {
        caps.types.iter().map(|t| t.r#type()).collect()
    }

    #[tokio::test]
    async fn crud_over_the_typed_api() {
        let f = fixture("grpc-crud");
        let token = f.login("*UserAgent*", "secret");
        let created = f.api.create_resource(req(pb::CreateResourceRequest {
            r#type: pb::ResourceType::Schedule as i32, text: WEEKLY.into(), validate_only: false }, &token))
            .await.unwrap().into_inner();
        assert_eq!(created.status(), pb::ConfigStatus::Ok, "{created:?}");
        assert!(created.reloaded);
        // the reload installed the new configuration (scheduler sees it)
        assert!(f.shared.read().unwrap().schedules.iter().any(|s| s.name == "WeeklyCycle"));

        let read = f.api.read_resource(req(pb::ReadResourceRequest { resource: schedule("WeeklyCycle") }, &token))
            .await.unwrap().into_inner();
        assert_eq!((read.status(), read.text.as_str()), (pb::ConfigStatus::Ok, WEEKLY));
        assert_eq!(read.resource.as_ref().unwrap().r#type(), pb::ResourceType::Schedule);

        let conflict = f.api.update_resource(req(pb::UpdateResourceRequest { resource: schedule("WeeklyCycle"),
            expected_sha256: "00".into(), text: WEEKLY.replace("sat", "sun"), validate_only: false }, &token))
            .await.unwrap().into_inner();
        assert_eq!((conflict.status(), conflict.sha256.as_str(), conflict.text.as_str()),
                   (pb::ConfigStatus::Conflict, read.sha256.as_str(), WEEKLY));

        let invalid = f.api.update_resource(req(pb::UpdateResourceRequest { resource: schedule("WeeklyCycle"),
            expected_sha256: read.sha256.clone(), text: "Schedule {\n  Name = WeeklyCycle\n  Nonsense = 1\n}\n".into(),
            validate_only: true }, &token)).await.unwrap().into_inner();
        assert_eq!(invalid.status(), pb::ConfigStatus::Invalid);
        let problem = invalid.problem.unwrap();
        assert_eq!((problem.line, problem.column), (3, 11));

        // a Job uses the schedule: REFERENCED
        let job = crate::config_edit::tests::JOB_A.replace("Name = a", "Name = b").replace("  Pool = Full\n", "  Pool = Full\n  Schedule = WeeklyCycle\n");
        let created_job = f.api.create_resource(req(pb::CreateResourceRequest {
            r#type: pb::ResourceType::Job as i32, text: job.clone(), validate_only: false }, &token)).await.unwrap().into_inner();
        assert_eq!(created_job.status(), pb::ConfigStatus::Ok, "{created_job:?}");
        let referenced = f.api.delete_resource(req(pb::DeleteResourceRequest { resource: schedule("WeeklyCycle"),
            expected_sha256: read.sha256.clone(), validate_only: false }, &token)).await.unwrap().into_inner();
        assert_eq!(referenced.status(), pb::ConfigStatus::Referenced);
        assert_eq!(referenced.referenced_by, vec![pb::ResourceRef { r#type: pb::ResourceType::Job as i32, name: "b".into() }]);

        let list = f.api.list_resources(req(pb::ListResourcesRequest { r#type: pb::ResourceType::Job as i32 }, &token))
            .await.unwrap().into_inner();
        let names: Vec<_> = list.resources.iter().map(|r| r.resource.as_ref().unwrap().name.as_str()).collect();
        assert_eq!(names, ["b", "a"]);
        assert_eq!(list.resources[0].sha256, crate::config_edit::sha256_hex(&job));

        let deleted_job = f.api.delete_resource(req(pb::DeleteResourceRequest {
            resource: Some(pb::ResourceRef { r#type: pb::ResourceType::Job as i32, name: "b".into() }),
            expected_sha256: created_job.sha256.clone(), validate_only: false }, &token)).await.unwrap().into_inner();
        assert_eq!(deleted_job.status(), pb::ConfigStatus::Ok);
        let deleted = f.api.delete_resource(req(pb::DeleteResourceRequest { resource: schedule("WeeklyCycle"),
            expected_sha256: read.sha256.clone(), validate_only: false }, &token)).await.unwrap().into_inner();
        assert_eq!((deleted.status(), deleted.old_sha256.as_str()), (pb::ConfigStatus::Ok, read.sha256.as_str()));
        assert!(!f.dir.join("bareos-dir.d/schedule/WeeklyCycle.conf").exists());

        // later: RenameResource, other types
        let err = f.api.rename_resource(req(pb::RenameResourceRequest::default(), &token)).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unimplemented);
        let err = f.api.list_resources(req(pb::ListResourcesRequest { r#type: pb::ResourceType::Console as i32 }, &token))
            .await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unimplemented);
        let err = f.api.read_resource(req(pb::ReadResourceRequest { resource: None }, &token)).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn acl_permission_denied_denied_and_capabilities_per_session() {
        let f = fixture("grpc-acl");
        let root = f.login("*UserAgent*", "secret");
        let created = f.api.create_resource(req(pb::CreateResourceRequest {
            r#type: pb::ResourceType::Schedule as i32, text: WEEKLY.into(), validate_only: false }, &root))
            .await.unwrap().into_inner();
        assert_eq!(created.status(), pb::ConfigStatus::Ok);

        // without a session
        let err = f.api.get_config_capabilities(Request::new(pb::ConfigCapabilitiesRequest {})).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);

        // default console: all seven types with all implemented operations
        let caps = f.api.get_config_capabilities(req(pb::ConfigCapabilitiesRequest {}, &root)).await.unwrap().into_inner();
        assert_eq!(caps.interface_version, 1);
        assert_eq!(types(&caps), [pb::ResourceType::Schedule, pb::ResourceType::Job, pb::ResourceType::Jobdefs, pb::ResourceType::Client,
            pb::ResourceType::Fileset, pb::ResourceType::Pool, pb::ResourceType::Storage]);
        assert!(caps.types.iter().all(|t| t.operations().collect::<Vec<_>>() == OPERATIONS));

        // no Command ACL configure: PERMISSION_DENIED, no types
        let viewer = f.login("viewer", "pw");
        let caps = f.api.get_config_capabilities(req(pb::ConfigCapabilitiesRequest {}, &viewer)).await.unwrap().into_inner();
        assert!(caps.types.is_empty());
        let err = f.api.read_resource(req(pb::ReadResourceRequest { resource: schedule("WeeklyCycle") }, &viewer))
            .await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        let err = f.api.delete_resource(req(pb::DeleteResourceRequest { resource: schedule("WeeklyCycle"),
            expected_sha256: created.sha256.clone(), validate_only: false }, &viewer)).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::PermissionDenied);

        // Schedule ACL without any allowed schedule: type missing, DENIED per name
        let nosched = f.login("noschedule", "pw");
        let caps = f.api.get_config_capabilities(req(pb::ConfigCapabilitiesRequest {}, &nosched)).await.unwrap().into_inner();
        assert_eq!(types(&caps), [pb::ResourceType::Job, pb::ResourceType::Jobdefs]);
        let denied = f.api.read_resource(req(pb::ReadResourceRequest { resource: schedule("WeeklyCycle") }, &nosched))
            .await.unwrap().into_inner();
        assert_eq!((denied.status(), denied.text.as_str()), (pb::ConfigStatus::Denied, ""));
        let denied = f.api.update_resource(req(pb::UpdateResourceRequest { resource: schedule("WeeklyCycle"),
            expected_sha256: created.sha256.clone(), text: WEEKLY.into(), validate_only: false }, &nosched))
            .await.unwrap().into_inner();
        assert_eq!(denied.status(), pb::ConfigStatus::Denied);

        // Job ACL per name: create of a denied name, list filtered
        let admin = f.login("admin", "pw");
        let job = crate::config_edit::tests::JOB_A.replace("Name = a", "Name = secret");
        let denied = f.api.create_resource(req(pb::CreateResourceRequest {
            r#type: pb::ResourceType::Job as i32, text: job, validate_only: true }, &admin)).await.unwrap().into_inner();
        assert_eq!(denied.status(), pb::ConfigStatus::Denied);
        std::fs::write(f.dir.join("bareos-dir.d/job/secret.conf"), crate::config_edit::tests::JOB_A.replace("Name = a", "Name = secret")).unwrap();
        let list = f.api.list_resources(req(pb::ListResourcesRequest { r#type: pb::ResourceType::Job as i32 }, &admin))
            .await.unwrap().into_inner();
        assert_eq!(list.resources.iter().map(|r| r.resource.as_ref().unwrap().name.as_str()).collect::<Vec<_>>(), ["a"]);
        let caps = f.api.get_config_capabilities(req(pb::ConfigCapabilitiesRequest {}, &admin)).await.unwrap().into_inner();
        assert_eq!(types(&caps), [pb::ResourceType::Schedule, pb::ResourceType::Job, pb::ResourceType::Jobdefs]);

        // Client ACL: only Client in the capabilities, DENIED for other names
        let clients = f.login("clients", "pw");
        let caps = f.api.get_config_capabilities(req(pb::ConfigCapabilitiesRequest {}, &clients)).await.unwrap().into_inner();
        assert_eq!(types(&caps), [pb::ResourceType::Client]);
        let client = |name: &str| Some(pb::ResourceRef { r#type: pb::ResourceType::Client as i32, name: name.into() });
        let read = f.api.read_resource(req(pb::ReadResourceRequest { resource: client("fd") }, &clients)).await.unwrap().into_inner();
        assert_eq!(read.status(), pb::ConfigStatus::Ok);
        assert_eq!(read.resource.unwrap().r#type(), pb::ResourceType::Client);
        let denied = f.api.create_resource(req(pb::CreateResourceRequest { r#type: pb::ResourceType::Client as i32,
            text: crate::config_edit::tests::CLIENT_TLS.into(), validate_only: true }, &clients)).await.unwrap().into_inner();
        assert_eq!(denied.status(), pb::ConfigStatus::Denied);
        // the Jobs using the client: REFERENCED
        let referenced = f.api.delete_resource(req(pb::DeleteResourceRequest { resource: client("fd"),
            expected_sha256: read.sha256, validate_only: true }, &clients)).await.unwrap().into_inner();
        assert_eq!(referenced.status(), pb::ConfigStatus::Referenced);
        let job = |name: &str| pb::ResourceRef { r#type: pb::ResourceType::Job as i32, name: name.into() };
        assert_eq!(referenced.referenced_by, vec![job("secret"), job("a")]);
    }
}
