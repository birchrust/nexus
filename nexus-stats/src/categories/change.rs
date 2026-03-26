//! Change detection and anomaly detection.

pub use crate::{
    CusumF32, CusumF32Builder, CusumF64, CusumF64Builder, CusumI128, CusumI128Builder,
    CusumI32, CusumI32Builder, CusumI64, CusumI64Builder, ErrorRateF32, ErrorRateF32Builder,
    ErrorRateF64, ErrorRateF64Builder, MultiGateF32, MultiGateF32Builder, MultiGateF64,
    MultiGateF64Builder, RobustZScoreF32, RobustZScoreF32Builder, RobustZScoreF64,
    RobustZScoreF64Builder, SaturationF32, SaturationF32Builder, SaturationF64,
    SaturationF64Builder, TrendAlertF32, TrendAlertF32Builder, TrendAlertF64,
    TrendAlertF64Builder, Verdict,
};

#[cfg(feature = "alloc")]
pub use crate::{
    MosumF32, MosumF32Builder, MosumF64, MosumF64Builder, MosumI128, MosumI128Builder,
    MosumI32, MosumI32Builder, MosumI64, MosumI64Builder,
};

#[cfg(any(feature = "std", feature = "libm"))]
pub use crate::{
    AdaptiveThresholdF32, AdaptiveThresholdF32Builder, AdaptiveThresholdF64,
    AdaptiveThresholdF64Builder, ShiryaevRobertsF64, ShiryaevRobertsF64Builder,
};
