-- 새 계정은 viewer가 기본이다. 이미 있던 계정은 방송을 만들 수 있었으므로
-- 모두 streamer로 올리고, 데모 시청자만 viewer로 되돌린다.
ALTER TABLE users
    ADD COLUMN role TEXT NOT NULL DEFAULT 'viewer'
        CHECK (role IN ('streamer', 'viewer'));

UPDATE users SET role = 'streamer';
UPDATE users SET role = 'viewer' WHERE is_demo AND username = 'demo-viewer';
