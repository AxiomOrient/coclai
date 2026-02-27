# CORE_API

아래는 `coclai` 공개 API를 사용 경로 기준으로 정리한 문서입니다.

## 0) 현재 상태 (Hook)

현재 공개 API에는 사용자 워크플로우용 `preHook/postHook` 등록 경로가 포함되어 있습니다.

- 현 상태: 라이프사이클 API(`run/setup/ask/close`) + `preHook/postHook` opt-in 실행 체인 사용 가능
- 정책: preHook 입력 변형 허용, hook 실패는 fail-open(에러 보고 + 메인 진행)
- 아키텍처: C-lite (공통 plugin core + artifact/web adapter 분리)
- 계약: `PluginContractVersion` 기반 cross-crate 호환성 게이트 활성
- 성능: hook off/on + adapter 오버헤드 micro-bench 게이트 활성
- 운영 문서: `README.md`, `Docs/ARCHITECTURE.md`, `Docs/SCHEMA_AND_CONTRACT.md`, `Docs/SECURITY.md`

## 1) 기본 경로: `coclai` (권장)

대부분의 사용자는 `coclai`만 사용하면 됩니다.

### 1.1 쉬운 경로 (초보자)

- `quick_run(cwd, prompt)`: connect -> run -> shutdown 원샷 실행
- `quick_run_with_profile(cwd, prompt, profile)`: 원샷 + profile override
- 실행 예제: `crates/coclai/examples/quick_run.rs`

### 1.2 명시적 경로 (전문가)

- `WorkflowConfig`: `cwd + client_config + run_profile`를 하나의 데이터 모델로 고정
- `WorkflowConfig::new(cwd)` 경로 정책:
  - 입력이 상대 경로면 프로세스 `current_dir` 기준 절대 경로로 즉시 정규화
  - 파일/디렉터리 존재 여부는 검사하지 않음(문자열 정규화만 수행)
  - `current_dir` 조회가 실패하면 입력 문자열을 그대로 유지
- `Workflow`: reusable client handle
  - `connect`, `run`, `run_with_profile`, `setup_session`, `shutdown`
- 실행 예제:
  - safe default: `crates/coclai/examples/workflow.rs`
  - privileged opt-in: `crates/coclai/examples/workflow_privileged.rs`

### 1.3 JSON-RPC 직접 경로

- `AppServer`: codex app-server JSON-RPC direct facade
- `rpc_methods::*`: 오타 없이 메서드 이름을 재사용하기 위한 상수 집합
- 실행 예제: `crates/coclai/examples/rpc_direct.rs` (`turn/completed`까지 대기 후 최종 텍스트 수집)

```rust
pub use ergonomic::{
    quick_run,
    quick_run_with_profile,
    QuickRunError,
    Workflow,
    WorkflowConfig,
};
pub use appserver::{methods as rpc_methods, AppServer};

pub use coclai_runtime::{
    ApprovalPolicy,
    Client,
    ClientConfig,
    ClientError,
    CompatibilityGuard,
    HookAction,
    HookAttachment,
    HookContext,
    HookIssue,
    HookIssueClass,
    HookPatch,
    HookPhase,
    HookReport,
    PluginContractVersion,
    PostHook,
    PreHook,
    PromptAttachment,
    PromptRunError,
    PromptRunParams,
    PromptRunResult,
    ReasoningEffort,
    RpcError,
    RpcErrorObject,
    RpcValidationMode,
    RuntimeError,
    RuntimeHookConfig,
    ServerRequest,
    ServerRequestRx,
    SandboxPolicy,
    SandboxPreset,
    SemVerTriplet,
    RunProfile,
    Session,
    SessionConfig,
    ThreadAgentMessageItemView,
    ThreadCommandExecutionItemView,
    ThreadLoadedListParams,
    ThreadLoadedListResponse,
    ThreadItemPayloadView,
    ThreadListParams,
    ThreadListResponse,
    ThreadListSortKey,
    ThreadReadParams,
    ThreadReadResponse,
    ThreadRollbackParams,
    ThreadRollbackResponse,
    ThreadTurnErrorView,
    ThreadTurnStatus,
    ThreadTurnView,
    ThreadItemType,
    ThreadItemView,
    ThreadView,
    DEFAULT_REASONING_EFFORT,
};

pub use coclai_runtime as runtime;
```

```rust
pub struct AppServer;
impl AppServer {
    pub async fn connect_default() -> Result<Self, ClientError>;
    pub async fn connect(config: ClientConfig) -> Result<Self, ClientError>;
    pub async fn request_json(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, RpcError>;
    pub async fn request_json_unchecked(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, RpcError>;
    pub async fn notify_json(&self, method: &str, params: serde_json::Value) -> Result<(), RuntimeError>;
    pub async fn notify_json_unchecked(&self, method: &str, params: serde_json::Value) -> Result<(), RuntimeError>;
    pub async fn take_server_requests(&self) -> Result<ServerRequestRx, RuntimeError>;
    pub async fn respond_server_request_ok(&self, approval_id: &str, result: serde_json::Value) -> Result<(), RuntimeError>;
    pub async fn respond_server_request_err(&self, approval_id: &str, err: RpcErrorObject) -> Result<(), RuntimeError>;
    pub async fn shutdown(&self) -> Result<(), RuntimeError>;
}
```

세션 라이프사이클 사용 순서는 `README.md`의 라이프사이클 섹션을 기준으로 합니다.

## 2) 확장 경로: `coclai_runtime`

저수준 제어(직접 RPC, 이벤트 스트림, approval 루프)가 필요할 때 사용합니다.

경계 규칙:

- 루트(`coclai_runtime`)는 실행/상태/정책의 핵심 타입만 재수출합니다.
- 내부 helper는 모듈 경로로만 접근합니다.
  - 예: `coclai_runtime::approvals::route_server_request`
  - 예: `coclai_runtime::rpc::map_rpc_error`
  - 예: `coclai_runtime::turn_output::AssistantTextCollector`

### 2.1 클라이언트

```rust
pub struct Client;
impl Client {
    pub async fn connect_default() -> Result<Self, ClientError>;
    pub async fn connect(config: ClientConfig) -> Result<Self, ClientError>;
    pub async fn run(
        &self,
        cwd: impl Into<String>,
        prompt: impl Into<String>
    ) -> Result<PromptRunResult, PromptRunError>;
    pub async fn run_with(&self, params: PromptRunParams) -> Result<PromptRunResult, PromptRunError>;
    pub async fn run_with_profile(
        &self,
        cwd: impl Into<String>,
        prompt: impl Into<String>,
        profile: RunProfile
    ) -> Result<PromptRunResult, PromptRunError>;
    pub async fn setup(&self, cwd: impl Into<String>) -> Result<Session, PromptRunError>;
    pub async fn setup_with_profile(
        &self,
        cwd: impl Into<String>,
        profile: RunProfile
    ) -> Result<Session, PromptRunError>;
    pub async fn start_session(&self, config: SessionConfig) -> Result<Session, PromptRunError>;
    pub async fn resume_session(
        &self,
        thread_id: &str,
        config: SessionConfig
    ) -> Result<Session, PromptRunError>;
    pub async fn interrupt_session_turn(&self, thread_id: &str, turn_id: &str) -> Result<(), RpcError>;
    pub async fn close_session(&self, thread_id: &str) -> Result<(), RpcError>;
    pub async fn shutdown(&self) -> Result<(), RuntimeError>;
}
```

### 2.2 세션

```rust
pub struct Session {
    pub thread_id: String,
    pub config: SessionConfig,
}

impl Session {
    pub fn is_closed(&self) -> bool;
    pub async fn ask(&self, prompt: impl Into<String>) -> Result<PromptRunResult, PromptRunError>;
    pub async fn ask_with_profile(
        &self,
        prompt: impl Into<String>,
        profile: RunProfile
    ) -> Result<PromptRunResult, PromptRunError>;
    pub async fn ask_with(&self, params: PromptRunParams) -> Result<PromptRunResult, PromptRunError>;
    pub async fn interrupt_turn(&self, turn_id: &str) -> Result<(), RpcError>;
    pub async fn close(&self) -> Result<(), RpcError>;
}
```

### 2.3 런타임

```rust
impl Runtime {
    pub async fn spawn_local(cfg: RuntimeConfig) -> Result<Self, RuntimeError>;
    pub async fn shutdown(&self) -> Result<(), RuntimeError>;

    pub async fn call_raw(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, RpcError>;
    pub async fn call_validated(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, RpcError>;
    pub async fn call_validated_with_mode(
        &self,
        method: &str,
        params: serde_json::Value,
        mode: RpcValidationMode
    ) -> Result<serde_json::Value, RpcError>;
    pub async fn notify_raw(&self, method: &str, params: serde_json::Value) -> Result<(), RuntimeError>;
    pub async fn notify_validated(&self, method: &str, params: serde_json::Value) -> Result<(), RuntimeError>;
    pub async fn notify_validated_with_mode(
        &self,
        method: &str,
        params: serde_json::Value,
        mode: RpcValidationMode
    ) -> Result<(), RuntimeError>;
    pub async fn thread_start(&self, p: ThreadStartParams) -> Result<ThreadHandle, RpcError>;
    pub async fn thread_resume(&self, thread_id: &str, p: ThreadStartParams) -> Result<ThreadHandle, RpcError>;
    pub async fn thread_fork(&self, thread_id: &str) -> Result<ThreadHandle, RpcError>;
    pub async fn thread_archive(&self, thread_id: &str) -> Result<(), RpcError>;
    pub async fn thread_read(&self, p: ThreadReadParams) -> Result<ThreadReadResponse, RpcError>;
    pub async fn thread_list(&self, p: ThreadListParams) -> Result<ThreadListResponse, RpcError>;
    pub async fn thread_loaded_list(&self, p: ThreadLoadedListParams) -> Result<ThreadLoadedListResponse, RpcError>;
    pub async fn thread_rollback(&self, p: ThreadRollbackParams) -> Result<ThreadRollbackResponse, RpcError>;
    pub async fn turn_interrupt(&self, thread_id: &str, turn_id: &str) -> Result<(), RpcError>;
    pub async fn run_prompt_simple(&self, cwd: impl Into<String>, prompt: impl Into<String>) -> Result<PromptRunResult, PromptRunError>;
    pub async fn run_prompt(&self, params: PromptRunParams) -> Result<PromptRunResult, PromptRunError>;
    pub async fn run_prompt_in_thread(&self, thread_id: &str, params: PromptRunParams) -> Result<PromptRunResult, PromptRunError>;

    pub fn subscribe_live(&self) -> tokio::sync::broadcast::Receiver<Envelope>;
    pub fn state_snapshot(&self) -> std::sync::Arc<RuntimeState>;
    pub fn metrics_snapshot(&self) -> RuntimeMetricsSnapshot;

    pub async fn take_server_request_rx(&self) -> Result<ServerRequestRx, RuntimeError>;
    pub async fn respond_approval_ok(&self, approval_id: &str, result: serde_json::Value) -> Result<(), RuntimeError>;
    pub async fn respond_approval_err(&self, approval_id: &str, err: RpcErrorObject) -> Result<(), RuntimeError>;
}
```

보장 규칙:

- `PromptRunParams::new`의 기본 effort는 `medium`
- `run_prompt` 경로는 첨부 파일 경로를 실행 전 검증
- `Session::close()` 이후 같은 핸들의 `ask/ask_with/interrupt_turn`은 로컬에서 즉시 거절
- `Session::close()`가 원격 `thread/archive` RPC 에러를 반환해도 로컬 핸들은 닫힌 상태로 유지
- `Session::close()` 재호출은 최초 close 결과를 캐시해 동일 결과를 반환
- `Client::connect()`의 호환성 검증 실패 경로는 `runtime.shutdown()` 실패를 더 이상 무시하지 않음
- `Runtime::shutdown()`은 dispatcher/supervisor join 실패를 `RuntimeError::Internal`로 전파

### 2.4 Hook 등록 API

`ClientConfig/RunProfile/SessionConfig`에서 hook를 등록할 수 있습니다.

```rust
pub struct ClientConfig {
    pub hooks: RuntimeHookConfig,
}
impl ClientConfig {
    pub fn with_hooks(self, hooks: RuntimeHookConfig) -> Self;
    pub fn with_pre_hook(self, hook: Arc<dyn PreHook>) -> Self;
    pub fn with_post_hook(self, hook: Arc<dyn PostHook>) -> Self;
}

pub struct RunProfile {
    pub hooks: RuntimeHookConfig,
}
impl RunProfile {
    pub fn with_hooks(self, hooks: RuntimeHookConfig) -> Self;
    pub fn with_pre_hook(self, hook: Arc<dyn PreHook>) -> Self;
    pub fn with_post_hook(self, hook: Arc<dyn PostHook>) -> Self;
}

pub struct SessionConfig {
    pub hooks: RuntimeHookConfig,
}
impl SessionConfig {
    pub fn with_hooks(self, hooks: RuntimeHookConfig) -> Self;
    pub fn with_pre_hook(self, hook: Arc<dyn PreHook>) -> Self;
    pub fn with_post_hook(self, hook: Arc<dyn PostHook>) -> Self;
}
```

## 3) 도메인 경로: `coclai_artifact`

문서/규칙 작업 파이프라인이 필요할 때 사용합니다.

```rust
pub trait ArtifactStore: Send + Sync {
    fn load_text(&self, artifact_id: &str) -> Result<String, StoreErr>;
    fn save_text(&self, artifact_id: &str, new_text: &str, meta: SaveMeta) -> Result<(), StoreErr>;
    fn get_meta(&self, artifact_id: &str) -> Result<ArtifactMeta, StoreErr>;
    fn set_meta(&self, artifact_id: &str, meta: ArtifactMeta) -> Result<(), StoreErr>;
}

pub struct ArtifactSessionManager;
impl ArtifactSessionManager {
    pub fn new(runtime: Runtime, store: std::sync::Arc<dyn ArtifactStore>) -> Self;
    pub fn new_with_adapter(
        adapter: std::sync::Arc<dyn ArtifactPluginAdapter>,
        store: std::sync::Arc<dyn ArtifactStore>
    ) -> Self;
    pub async fn open(&self, artifact_id: &str) -> Result<ArtifactSession, DomainError>;
    pub async fn run_task(&self, spec: ArtifactTaskSpec) -> Result<ArtifactTaskResult, DomainError>;
}
```

호환성 규칙:

- `ArtifactPluginAdapter::plugin_contract_version()`은 기본 `PluginContractVersion::CURRENT`
- major 불일치 시 `DomainError::IncompatibleContract` 반환

## 4) 웹 경로: `coclai_web`

세션/턴/SSE/승인 브리지가 필요할 때 사용합니다.

```rust
pub struct WebAdapter;
impl WebAdapter {
    pub async fn spawn(runtime: Runtime, config: WebAdapterConfig) -> Result<Self, WebError>;
    pub async fn spawn_with_adapter(
        adapter: std::sync::Arc<dyn WebPluginAdapter>,
        config: WebAdapterConfig
    ) -> Result<Self, WebError>;
    pub async fn create_session(
        &self,
        tenant_id: &str,
        request: CreateSessionRequest
    ) -> Result<CreateSessionResponse, WebError>;
    pub async fn create_turn(
        &self,
        tenant_id: &str,
        session_id: &str,
        request: CreateTurnRequest
    ) -> Result<CreateTurnResponse, WebError>;
    pub async fn close_session(
        &self,
        tenant_id: &str,
        session_id: &str
    ) -> Result<CloseSessionResponse, WebError>;
    pub async fn subscribe_session_events(
        &self,
        tenant_id: &str,
        session_id: &str
    ) -> Result<tokio::sync::broadcast::Receiver<Envelope>, WebError>;
    pub async fn subscribe_session_approvals(
        &self,
        tenant_id: &str,
        session_id: &str
    ) -> Result<tokio::sync::broadcast::Receiver<ServerRequest>, WebError>;
    pub async fn post_approval(
        &self,
        tenant_id: &str,
        session_id: &str,
        approval_id: &str,
        payload: ApprovalResponsePayload
    ) -> Result<(), WebError>;
}
```

`WebAdapter::spawn` 제약:
- 하나의 `Runtime` 인스턴스에는 하나의 `WebAdapter`만 바인딩할 수 있습니다.
- 같은 `Runtime`으로 두 번째 `spawn`을 호출하면 `WebError::AlreadyBound`를 반환합니다.
- `plugin_contract_version` major 불일치 시 `WebError::IncompatibleContract`를 반환합니다.
