// ForgeClaw WebUI API 类型定义
// 对齐后端 API 契约（见任务说明）。

/** 用户身份。 */
export interface User {
  id: string
  name: string
}

/** `POST /api/auth/login` 响应。 */
export interface LoginResponse {
  ok: boolean
  user: User
  /** 一次性 WebSocket ticket（首连用）。 */
  ticket: string
}

/** `GET /api/auth/ticket` 响应。 */
export interface TicketResponse {
  ticket: string
}

/** 工具调用记录。 */
export interface ToolCall {
  id: string
  tool: string
  input: unknown
}

/** 工具执行结果。 */
export interface ToolResult {
  output: string
  duration_ms: number
  error?: string | null
}

/** Complete 帧汇总的工具调用记录（含结果）。 */
export interface ToolCallRecord {
  id: string
  name: string
  input?: unknown
  result: ToolResult
}

/**
 * 会话消息（discriminated union，按角色区分）。
 * - User: 用户输入文本
 * - Assistant: 模型回复（含可能的工具调用列表）
 * - Tool: 工具调用与对应结果（二元组）
 */
export type Message =
  | { User: string }
  | { Assistant: { text: string; tool_calls: ToolCall[] } }
  | { Tool: [ToolCall, ToolResult] }

/** `GET /api/sessions` 列表项（摘要）。 */
export interface SessionSummary {
  id: string
  created_at?: string
  message_count?: number
  [key: string]: unknown
}

/** `GET /api/sessions/:id` 详情。 */
export interface SessionDetail {
  id: string
  created_at?: string
  messages: Message[]
  [key: string]: unknown
}

/** `GET /api/tools` 列表项。 */
export interface ToolInfo {
  name: string
  description: string
  parameters?: unknown
  [key: string]: unknown
}

/**
 * WebSocket `/ws/chat` 接收帧（OrchestratorEvent）。
 * 客户端发送 `{message, session_id?}`，按 type 分发以下帧。
 */
export type OrchestratorEvent =
  | { type: 'delta'; text: string }
  | { type: 'tool_call_start'; id: string; name: string; input: unknown }
  | { type: 'tool_result'; id: string; name: string; result: ToolResult }
  | { type: 'complete'; text: string; tool_calls: ToolCallRecord[] }
  | { type: 'error'; message: string }

/** WS 客户端发送帧。 */
export interface WsChatRequest {
  message: string
  session_id?: string
}
