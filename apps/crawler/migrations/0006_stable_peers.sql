CREATE TABLE IF NOT EXISTS stable_peers (
    ip INET NOT NULL,
    port INT NOT NULL,
    metadata_provided_count INT DEFAULT 1,
    first_seen TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (ip, port)
);
