#![cfg_attr(docsrs, feature(doc_cfg))]

/// Outer doc.
/** Block doc. */
#[unsafe(no_mangle)]
#[doc = "explicit"]
#[cfg_attr(test, derive(Debug))]
pub extern "C" fn exported() {}
