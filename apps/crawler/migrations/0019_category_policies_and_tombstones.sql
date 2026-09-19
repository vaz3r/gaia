-- Migration 0019: Category Policies and Anti-Recrawl Tombstones
-- Enables dynamic category toggles and permanent anti-recrawl filtering

CREATE TABLE IF NOT EXISTS category_policies (
    category VARCHAR(64) PRIMARY KEY,
    is_enabled BOOLEAN NOT NULL DEFAULT true,
    auto_purge BOOLEAN NOT NULL DEFAULT false,
    description TEXT,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT now()
);

-- Seed canonical categories (default all enabled)
INSERT INTO category_policies (category, is_enabled, auto_purge, description) VALUES
    ('Adult', true, false, 'Adult / NSFW media content'),
    ('Movies', true, false, 'Feature films, cinema releases, and movies'),
    ('Television', true, false, 'TV shows, series, and broadcast media'),
    ('Music', true, false, 'Lossless, MP3, and audio recordings'),
    ('Books & Learning', true, false, 'E-books, educational, technical, and academic materials'),
    ('Anime', true, false, 'Anime episodes, seasons, movies, and specials'),
    ('Games', true, false, 'PC and console video games'),
    ('Applications', true, false, 'Software applications, utilities, and operating systems'),
    ('Audiobooks', true, false, 'Narrated audiobooks and spoken-word audio'),
    ('Documentaries', true, false, 'Documentary features and non-fiction series'),
    ('Other', true, false, 'Uncategorized releases and miscellaneous swarms')
ON CONFLICT (category) DO NOTHING;

-- Permanent Anti-Recrawl Tombstones
CREATE TABLE IF NOT EXISTS blocked_infohashes (
    infohash BYTEA PRIMARY KEY,
    category VARCHAR(64) NOT NULL,
    reason VARCHAR(64) NOT NULL DEFAULT 'category_disabled',
    blocked_at TIMESTAMP WITH TIME ZONE DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_blocked_infohashes_category ON blocked_infohashes (category);
CREATE INDEX IF NOT EXISTS idx_blocked_infohashes_blocked_at ON blocked_infohashes (blocked_at);
