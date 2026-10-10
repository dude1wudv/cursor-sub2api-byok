CREATE TABLE route_diagnostics (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at_ms INTEGER NOT NULL,
    request_id TEXT,
    conversation_id TEXT,
    parent_request_id TEXT,
    parent_tool_call_id TEXT,
    method TEXT NOT NULL,
    path TEXT NOT NULL,
    route TEXT NOT NULL,
    stage TEXT NOT NULL,
    http_status INTEGER NOT NULL,
    duration_ms INTEGER NOT NULL
);
CREATE INDEX route_diagnostics_request ON route_diagnostics(request_id, id);
