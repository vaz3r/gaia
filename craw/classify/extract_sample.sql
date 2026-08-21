SELECT jsonb_build_object(
  'id', encode(t.infohash, 'hex'),
  'name', t.name,
  'file_count', t.file_count,
  'total_size', t.total_size,
  'top_dirs', (SELECT jsonb_agg(DISTINCT (f->'path'->>0)) FROM jsonb_array_elements(t.files) f WHERE jsonb_typeof(f->'path') = 'array'),
  'regex_explicit', (
    t.name ~* 'xxx|pthc|pedo|incest|teen|jailbait|loli|porn|cam|onlyfans|milf|babe|nude|sex.?video|sex.?tape|anal|blowjob|fuck'
    OR EXISTS (SELECT 1 FROM jsonb_array_elements(t.files) f WHERE (f->'path'->>0) ~* 'xxx|pthc|pedo|porn|onlyfans' OR (f->>'length')::text = '' )
  )
)
FROM torrents t
WHERE t.name IS NOT NULL AND t.name <> '' AND t.name <> '[unknown]'
ORDER BY random()
LIMIT 1000;
