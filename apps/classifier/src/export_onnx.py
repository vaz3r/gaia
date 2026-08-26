#!/usr/bin/env python3
import sys
import logging
from pathlib import Path
import subprocess

logging.basicConfig(level=logging.INFO, stream=sys.stderr, format='%(asctime)s %(levelname)s %(message)s')
logger = logging.getLogger('export_onnx')

MODEL_DIR = Path("data/models/transformer/single_stage")

def main():
    logger.info("Exporting to ONNX using Optimum...")
    
    cmd = [
        "optimum-cli", "export", "onnx",
        "--model", str(MODEL_DIR / "model"),
        "--task", "text-classification",
        "--library-name", "transformers",
        "--optimize", "O1",
        str(MODEL_DIR)
    ]
    
    try:
        subprocess.run(cmd, check=True)
    except subprocess.CalledProcessError as e:
        logger.error("Optimum export failed: %s", e)
        sys.exit(1)
        
    onnx_path = MODEL_DIR / "model.onnx"
    quant_path = MODEL_DIR / "model_int8.onnx"
    
    logger.info("Quantizing to int8 using ONNXRuntime...")
    try:
        from onnxruntime.quantization import quantize_dynamic, QuantType
        quantize_dynamic(
            str(onnx_path),
            str(quant_path),
            weight_type=QuantType.QInt8,
        )
        logger.info("Quantization successful. Int8 model saved at: %s", quant_path)
    except Exception as e:
        logger.error("Quantization failed: %s", e)

if __name__ == "__main__":
    main()
