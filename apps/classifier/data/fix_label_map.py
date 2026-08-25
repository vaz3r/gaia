import json

with open("labeled.jsonl", "r") as f:
    lines = f.readlines()

print("# Gold-standard manual labels: index -> (category, keep)")
print("# Taxonomy: Movies, Television, Games, Music, Applications, Anime, Documentaries, Other, Porn")
print("L = {")

for i, line in enumerate(lines):
    data = json.loads(line)
    cat = data.get("label_category")
    keep = data.get("label_keep")
    
    # Just formatted as it was originally, but simpler
    print(f"    {i}:('{cat}',{keep}),")

print("}")
