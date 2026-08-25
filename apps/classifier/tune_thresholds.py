import json
import numpy as np
from sklearn.model_selection import train_test_split
from sklearn.metrics import f1_score
from src.backends.transformer_onnx_backend import TransformerOnnxBackend
from src.core.text_builder import build_input_text
import joblib

def load_data():
    records = []
    with open('data/labeled.jsonl') as f:
        for line in f:
            records.append(json.loads(line))
            
    # Stratified split to get 15% validation set
    labels = [r.get('label_category', r.get('true_category')) for r in records]
    _, val_records = train_test_split(records, test_size=0.15, random_state=42, stratify=labels)
    return val_records

def main():
    val_records = load_data()
    print(f"Loaded {len(val_records)} validation records.")
    
    texts = [build_input_text(r, {}) for r in val_records]
    y_true = [r.get('label_category', r.get('true_category')) for r in val_records]
    
    print("Loading models...")
    s1_backend = TransformerOnnxBackend(
        model_path='data/models/transformer/stage1/model_int8.onnx',
        tokenizer_path='data/models/transformer/stage1/tokenizer',
        max_length=128
    )
    s2_backend = TransformerOnnxBackend(
        model_path='data/models/transformer/stage2/model_int8.onnx',
        tokenizer_path='data/models/transformer/stage2/tokenizer',
        max_length=128
    )
    
    s1_le = joblib.load('data/models/transformer/stage1/label_encoder.joblib')
    s2_le = joblib.load('data/models/transformer/stage2/label_encoder.joblib')
    s1_classes = s1_le.classes_
    s2_classes = s2_le.classes_
    
    print("Running inference...")
    s1_probs, _ = s1_backend.predict(texts, batch_size=32)
    s2_probs, s2_preds = s2_backend.predict(texts, batch_size=32)
    
    best_f1 = 0
    best_thresh_other = 0
    best_thresh_porn = 0
    
    thresholds_other = [0.5, 0.6, 0.7, 0.8, 0.9, 0.95, 0.98, 0.99]
    thresholds_porn = [0.5, 0.6, 0.7, 0.8, 0.9, 0.95]
    
    for t_other in thresholds_other:
        for t_porn in thresholds_porn:
            y_pred = []
            for i in range(len(val_records)):
                # Stage 1 logic
                probs = s1_probs[i]
                max_idx = np.argmax(probs)
                max_cat = s1_classes[max_idx]
                max_prob = probs[max_idx]
                
                category = "Unknown"
                
                if max_cat == "Porn" and max_prob >= t_porn:
                    category = "Porn"
                elif max_cat == "Other" and max_prob >= t_other:
                    category = "Other"
                else:
                    # Stage 2
                    category = s2_classes[int(s2_preds[i])]
                
                y_pred.append(category)
                
            macro_f1 = f1_score(y_true, y_pred, average='macro')
            if macro_f1 > best_f1:
                best_f1 = macro_f1
                best_thresh_other = t_other
                best_thresh_porn = t_porn
                
    print(f"Best Validation Macro F1: {best_f1:.4f}")
    print(f"Best Thresholds - Other: {best_thresh_other}, Porn: {best_thresh_porn}")

if __name__ == "__main__":
    main()
