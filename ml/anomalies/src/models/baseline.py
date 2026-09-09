import numpy as np
import pandas as pd
from typing import Dict, Any, Tuple

class StatisticalBaselineDetector:
    """
    Statistical anomaly detector utilizing rolling Exponentially Weighted Moving Averages (EWMA)
    and Median Absolute Deviation (MAD) for robust single-metric confidence bands.
    """
    def __init__(self, span: int = 24, z_threshold: float = 3.0):
        self.span = span
        self.z_threshold = z_threshold

    def compute_ewma_bands(self, series: pd.Series) -> pd.DataFrame:
        """
        Compute rolling EWMA mean and standard deviation bands.
        """
        ewma_mean = series.ewm(span=self.span, adjust=False).mean()
        ewma_std = series.ewm(span=self.span, adjust=False).std().fillna(1e-6)
        
        upper_band = ewma_mean + self.z_threshold * ewma_std
        lower_band = ewma_mean - self.z_threshold * ewma_std

        is_anomaly = (series > upper_band) | (series < lower_band)
        
        return pd.DataFrame({
            "value": series,
            "mean": ewma_mean,
            "upper": upper_band,
            "lower": lower_band,
            "is_anomaly": is_anomaly,
        })

    def compute_mad_zscore(self, series: pd.Series) -> pd.Series:
        """
        Compute robust Modified Z-Score using Median Absolute Deviation:
        M_i = 0.6745 * (x_i - median) / MAD
        """
        median = series.median()
        mad = np.median(np.abs(series - median))
        if mad == 0:
            mad = 1e-6
        mod_z = 0.6745 * (series - median) / mad
        return mod_z
