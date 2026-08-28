import re
with open('apps/crawler/src/dht/mod.rs', 'r') as f:
    text = f.read()

if "pub mod bep51;" not in text:
    text = "pub mod bep51;\n" + text

with open('apps/crawler/src/dht/mod.rs', 'w') as f:
    f.write(text)
