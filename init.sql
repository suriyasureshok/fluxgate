CREATE TABLE tiers (
    id VARCHAR(50) PRIMARY KEY,
    description TEXT,
    token_capacity DOUBLE PRECISION NOT NULL,
    refill_rate DOUBLE PRECISION NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

INSERT INTO tiers (id, description, token_capacity, refill_rate) VALUES 
('free', 'Standard limits for hobbyists', 5000.0, 100.0),
('pro', 'High throughput for paying customers', 50000.0, 1000.0);

CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email VARCHAR(255) UNIQUE NOT NULL,
    password_hash VARCHAR(255) NOT NULL, -- ADDED: Required for Login/Signup
    tier_id VARCHAR(50) REFERENCES tiers(id) ON DELETE RESTRICT,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE api_keys (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    key_hash VARCHAR(64) UNIQUE NOT NULL, 
    key_prefix VARCHAR(15) NOT NULL, 
    status VARCHAR(20) DEFAULT 'active' CHECK (status IN ('active', 'revoked', 'suspended')),
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    last_used_at TIMESTAMP WITH TIME ZONE
);

CREATE INDEX idx_api_keys_hash ON api_keys(key_hash);