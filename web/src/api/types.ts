export interface User {
  id: string
  name: string
  token: string
}

export interface LoginRequest {
  name: string
  token: string
}

export interface LoginResponse {
  ok: boolean
  user: User
}

export interface ChatRequest {
  message: string
  session_id?: string
}

export interface ToolCallRecord {
  id: string
  name: string
  result: ToolResult
}

export interface ChatResponse {
  session_id: string
  text: string
  tool_calls: ToolCallRecord[]
}

export interface SessionSummary {
  id: string
  created_at: string
  message_count: number
}

export interface AssistantMsg {
  text: string
  tool_calls: ToolCall[]
}

export interface ToolCall {
  id: string
  tool: string
  input: Record<string, unknown>
}

export interface ToolResult {
  output: string
  error: string | null
  duration_ms: number
}

export type Message =
  | { User: string }
  | { Assistant: AssistantMsg }
  | { Tool: [ToolCall, ToolResult] }

export interface SessionDetail {
  id: string
  created_at: string
  messages: Message[]
}

export interface ToolInfo {
  name: string
  description: string
  parameters: Record<string, unknown>
}

export interface ToolsResponse {
  tools: ToolInfo[]
}

export interface Section {
  id: string
  title: string
  level: 'critical' | 'confirm' | 'allow'
  enabled: boolean
  order: number
  body: string
}

export interface CompilePromptRequest {
  profile: string
}

export interface CompilePromptResponse {
  prompt: string
}

export type OrchestratorEvent =
  | { type: 'delta'; text: string }
  | { type: 'tool_call_start'; name: string; input: Record<string, unknown> }
  | { type: 'tool_result'; name: string; result: ToolResult }
  | { type: 'complete'; text: string; tool_calls: ToolCallRecord[] }
  | { type: 'error'; message: string }
