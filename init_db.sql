CREATE TABLE IF NOT EXISTS transcode_files (
    "path" VARCHAR NOT NULL UNIQUE,
    "status" VARCHAR NOT NULL DEFAULT 'pending',
    created_on BIGINT NOT NULL,
    updated_on BIGINT NOT NULL,
    error_message VARCHAR,
    file_size BIGINT NOT NULL,
    ffprobe_info VARCHAR
)