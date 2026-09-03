#!/usr/bin/env python3
import argparse
import sys
import logging
from pathlib import Path
import subprocess

logging.basicConfig(level=logging.INFO, stream=sys.stderr, format='%(asctime)s %(levelname)s %(message)s')
logger = logging.getLogger('export_onnx')

DEFAULT_MODEL_DIR = Path("data/models/transformer/single_stage")


def main():
    parser = argparse.ArgumentParser(description="Export transformer model to ONNX INT8")
    parser.add_argument(
        "--model_dir",
        default=str(DEFAULT_MODEL_DIR),
        help="Directory containing model/ checkpoint (default: data/models/transformer/single_stage)",
    )
    args = parser.parse_args()

    model_dir = Path(args.model_dir)
    logger.info("Exporting to ONNX using Optimum from %s...", model_dir)
    
    cmd = [
        "optimum-cli", "export", "onnx",
        "--model", str(model_dir / "model"),
        "--task", "text-classification",
        "--library-name", "transformers",
        "--optimize", "O1",
        str(model_dir)
    ]
    
    try:
        subprocess.run(cmd, check=True)
    except subprocess.CalledProcessError as e:
        logger.error("Optimum export failed: %s", e)
        sys.exit(1)
        
    onnx_path = model_dir / "model.onnx"
    quant_path = model_dir / "model_int8.onnx"
    
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
