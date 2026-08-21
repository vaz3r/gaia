#!/bin/bash
set -e

OLLAMA_HOST="${OLLAMA_HOST:-http://ollama:11434}"
MODEL_NAME="${MODEL_NAME:-gemma4-e2b-q4km}"

echo "Waiting for ollama to be ready..."
python3 -c "
import urllib.request, time
while True:
    try:
        urllib.request.urlopen('$OLLAMA_HOST/api/tags', timeout=2)
        break
    except Exception:
        time.sleep(2)
"
echo "Ollama is ready."

# Check if model already exists
EXISTS=$(python3 -c "
import urllib.request, json
resp = urllib.request.urlopen('$OLLAMA_HOST/api/tags', timeout=5)
models = json.loads(resp.read())['models']
names = [m['name'] for m in models]
print('yes' if any('$MODEL_NAME' in n for n in names) else 'no')
")

if [ "$EXISTS" = "no" ]; then
    echo "Model $MODEL_NAME not found, creating..."
    python3 -c "
import json, urllib.request, sys, os

model_name = os.environ['MODEL_NAME']
ollama_host = os.environ['OLLAMA_HOST']
gguf_path = '/models/gemma-4-E2B-it-Q4_K_M.gguf'

modelfile = 'FROM ' + gguf_path + '\nTEMPLATE \"\"\"<start_of_turn>user\n{{ .Prompt }}<end_of_turn>\n<start_of_turn>model\n\"\"\"\nPARAMETER temperature 0\nPARAMETER num_predict 64\n'

data = json.dumps({'name': model_name, 'modelfile': modelfile}).encode()
req = urllib.request.Request(
    ollama_host + '/api/create',
    data=data,
    headers={'Content-Type': 'application/json'},
)
resp = urllib.request.urlopen(req, timeout=600)
for line in resp:
    obj = json.loads(line)
    if 'error' in obj:
        print('ERROR: ' + obj['error'], file=sys.stderr)
        sys.exit(1)
    if obj.get('status'):
        print(obj['status'])
print('Model created.')
"
else
    echo "Model $MODEL_NAME already exists, skipping creation."
fi

echo "Running inference..."
pip install --no-cache-dir requests -q
python3 /app/infer.py \
    --model "$MODEL_NAME" \
    --sample /data/sample.json \
    --output /results/predictions.json \
    --ollama-host "$OLLAMA_HOST"

echo "Running evaluation..."
python3 /app/evaluate.py \
    --predictions /results/predictions.json \
    --labels /data/eval_labels.json \
    --subset eval \
    | tee /results/eval_report.txt

echo "Done. Results in /results/"
