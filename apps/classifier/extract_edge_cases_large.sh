#!/bin/bash
export SSHPASS='rosrtdz@1995'
sshpass -e ssh -o StrictHostKeyChecking=no core@workspace-production << 'EOF'
docker exec craw-db psql -U crawler -d craw -t -c "
SELECT row_to_json(t) FROM (
    SELECT infohash, name, file_count, total_size, files
    FROM torrents
    WHERE name ILIKE '%.rar%' OR name ILIKE '%.7z%' OR name ILIKE '%.zip%' OR name ILIKE '%repack%' OR name ILIKE '%fitgirl%' OR name ILIKE '%1080p%' OR name ILIKE '%720p%'
    ORDER BY RANDOM()
    LIMIT 3000
) t;
" > /tmp/edge_cases_large.jsonl
EOF
sshpass -e scp core@workspace-production:/tmp/edge_cases_large.jsonl data/edge_cases_large.jsonl
