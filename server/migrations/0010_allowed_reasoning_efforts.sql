ALTER TABLE model_configs ADD COLUMN allowed_reasoning_efforts_json TEXT NOT NULL DEFAULT '["low","medium","high","xhigh","max"]';
