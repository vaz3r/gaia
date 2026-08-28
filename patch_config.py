import re
with open('apps/crawler/src/config.rs', 'r') as f:
    text = f.read()

text = text.replace("tcp_timeout_secs: 5,", "tcp_timeout_secs: 2,")
text = text.replace("utp_timeout_secs: 10,", "utp_timeout_secs: 4,")
text = text.replace("metadata_timeout_secs: 10,", "metadata_timeout_secs: 4,")
text = text.replace("global_fetch_limit: 1200,", "global_fetch_limit: 600,")

with open('apps/crawler/src/config.rs', 'w') as f:
    f.write(text)

with open('apps/crawler/config/default.toml', 'r') as f:
    text = f.read()

text = text.replace("tcp_timeout_secs = 5", "tcp_timeout_secs = 2")
text = text.replace("utp_timeout_secs = 10", "utp_timeout_secs = 4")
text = text.replace("metadata_timeout_secs = 10", "metadata_timeout_secs = 4")
text = text.replace("global_fetch_limit = 1200", "global_fetch_limit = 600")

with open('apps/crawler/config/default.toml', 'w') as f:
    f.write(text)
