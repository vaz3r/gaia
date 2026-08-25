#!/bin/bash
sshpass -p 'rosrtdz@1995' ssh -o StrictHostKeyChecking=no core@workspace-production << 'EOF' > remote_legit.jsonl
docker exec craw-db psql -U crawler -d craw -t -c "
SELECT row_to_json(t) FROM (
    SELECT infohash, name, file_count, total_size, files, 'Movies' as label_category, true as label_keep
    FROM torrents
    WHERE (name ILIKE '%1080p%' OR name ILIKE '%720p%')
      AND name NOT ILIKE '%xxx%' AND name NOT ILIKE '%porn%'
    LIMIT 200
) t
UNION ALL
SELECT row_to_json(t) FROM (
    SELECT infohash, name, file_count, total_size, files, 'Music' as label_category, true as label_keep
    FROM torrents
    WHERE (name ILIKE '%flac%' OR name ILIKE '%mp3%')
      AND name NOT ILIKE '%xxx%' AND name NOT ILIKE '%porn%'
    LIMIT 200
) t
UNION ALL
SELECT row_to_json(t) FROM (
    SELECT infohash, name, file_count, total_size, files, 'Games' as label_category, true as label_keep
    FROM torrents
    WHERE (name ILIKE '%repack%' OR name ILIKE '%iso%')
      AND name NOT ILIKE '%xxx%' AND name NOT ILIKE '%porn%'
    LIMIT 200
) t
UNION ALL
SELECT row_to_json(t) FROM (
    SELECT infohash, name, file_count, total_size, files, 'Television' as label_category, true as label_keep
    FROM torrents
    WHERE (name ILIKE '%s0%' OR name ILIKE '%season%')
      AND name NOT ILIKE '%xxx%' AND name NOT ILIKE '%porn%'
    LIMIT 200
) t;
"
EOF
