#!/bin/bash
export SSHPASS='rosrtdz@1995'
sshpass -e ssh -o StrictHostKeyChecking=no core@workspace-production << 'EOF'
docker exec craw-db psql -U crawler -d craw -t -c "
SELECT row_to_json(t) FROM (
    SELECT infohash, name, file_count, total_size, files
    FROM torrents
    WHERE name ILIKE '%.rar%' OR name ILIKE '%.7z%' OR name ILIKE '%.zip%' OR name ILIKE '%repack%' OR name ILIKE '%fitgirl%'
    ORDER BY RANDOM()
    LIMIT 300
) t;
" > /tmp/edge_cases.jsonl
EOF
sshpass -e scp core@workspace-production:/tmp/edge_cases.jsonl data/edge_cases.jsonl
