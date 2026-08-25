ALTER TABLE verification_jobs DROP CONSTRAINT IF EXISTS valid_status;
ALTER TABLE verification_jobs ADD CONSTRAINT valid_status
    CHECK (status IN ('pending', 'verifying', 'verified', 'failed', 'dead'));
