#!/bin/bash
sshpass -p 'rosrtdz@1995' ssh -o StrictHostKeyChecking=no core@workspace-production << 'EOF' > remote_porn.jsonl
docker exec craw-db psql -U crawler -d craw -t -c "
SELECT row_to_json(t) FROM (
    SELECT infohash, name, file_count, total_size, files, 'Porn' as label_category, true as label_keep
    FROM torrents
    WHERE name ILIKE '%jav%' 
       OR name ILIKE '%onlyfans%' 
       OR name ILIKE '%hentai%' 
       OR name ILIKE '%xxx%' 
       OR name ILIKE '%porn%'
    LIMIT 200
) t;
"
EOF
