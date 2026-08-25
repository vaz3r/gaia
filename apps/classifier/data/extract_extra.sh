#!/bin/bash
sshpass -p 'rosrtdz@1995' ssh -o StrictHostKeyChecking=no core@workspace-production << 'EOF' > remote_extra.jsonl
docker exec craw-db psql -U crawler -d craw -t -c "
SELECT row_to_json(t) FROM (
    SELECT infohash, name, file_count, total_size, files, 'Movies' as label_category, true as label_keep
    FROM torrents
    WHERE (name ILIKE '%1080p%' OR name ILIKE '%720p%' OR name ILIKE '%2160p%')
      AND (name ILIKE '%bluray%' OR name ILIKE '%web-dl%' OR name ILIKE '%remux%')
      AND name NOT ILIKE '%xxx%' AND name NOT ILIKE '%porn%'
    LIMIT 100
) t
UNION ALL
SELECT row_to_json(t) FROM (
    SELECT infohash, name, file_count, total_size, files, 'Music' as label_category, true as label_keep
    FROM torrents
    WHERE (name ILIKE '%flac%' OR name ILIKE '%mp3%' OR name ILIKE '%320kbps%' OR name ILIKE '%discography%')
      AND name NOT ILIKE '%xxx%' AND name NOT ILIKE '%porn%'
    LIMIT 100
) t
UNION ALL
SELECT row_to_json(t) FROM (
    SELECT infohash, name, file_count, total_size, files, 'Documentaries' as label_category, true as label_keep
    FROM torrents
    WHERE (name ILIKE '%documentary%' OR name ILIKE '%bbc%' OR name ILIKE '%national geographic%' OR name ILIKE '%pbs%')
      AND name NOT ILIKE '%xxx%' AND name NOT ILIKE '%porn%'
    LIMIT 100
) t;
"
EOF
