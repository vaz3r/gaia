#!/bin/bash
sshpass -p 'rosrtdz@1995' ssh -o StrictHostKeyChecking=no core@workspace-production << 'EOF' > remote_porn_western.jsonl
docker exec craw-db psql -U crawler -d craw -t -c "
SELECT row_to_json(t) FROM (
    SELECT infohash, name, file_count, total_size, files, 'Porn' as label_category, false as label_keep
    FROM torrents
    WHERE (
        name ILIKE '%brazzers%' OR 
        name ILIKE '%bangbros%' OR 
        name ILIKE '%naughty%' OR 
        name ILIKE '%evilangel%' OR 
        name ILIKE '%realitykings%' OR 
        name ILIKE '%mofos%' OR 
        name ILIKE '%porn%' OR
        name ILIKE '%xxx%' OR
        name ILIKE '%x-art%' OR
        name ILIKE '%tushy%' OR
        name ILIKE '%blacked%' OR
        name ILIKE '%vixen%' OR
        name ILIKE '%milf%' OR
        name ILIKE '%teenslovehugecocks%' OR
        name ILIKE '%hustler%' OR
        name ILIKE '%peter north%' OR
        name ILIKE '%bigtits%'
    )
    LIMIT 1000
) t;
"
EOF
