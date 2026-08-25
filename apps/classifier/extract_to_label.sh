#!/bin/bash
export SSHPASS='rosrtdz@1995'
sshpass -e ssh -o StrictHostKeyChecking=no core@workspace-production << 'EOF'
docker exec craw-db psql -U crawler -d craw -t -c "
SELECT row_to_json(t) FROM (
    SELECT infohash, name, file_count, total_size, files
    FROM torrents
    WHERE name ILIKE '%.rar%' OR name ILIKE '%.7z%' OR name ILIKE '%.zip%'
    ORDER BY RANDOM()
    LIMIT 150
) t;
" > /tmp/to_label.jsonl
EOF
sshpass -e scp core@workspace-production:/tmp/to_label.jsonl data/to_label.jsonl
