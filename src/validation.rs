use crate::{GraphError, PropertyValue};

pub(crate) fn identifier(value: &str, max: usize, name: &str) -> Result<(), GraphError> {
    if value.is_empty()
        || value.len() > max
        || value.trim() != value
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
        })
    {
        return Err(GraphError::Invalid(format!("{name} is invalid")));
    }
    Ok(())
}

pub(crate) fn property_key(value: &str) -> Result<(), GraphError> {
    identifier(value, 128, "property key")
}

pub(crate) fn property_value(value: &PropertyValue) -> Result<(), GraphError> {
    match value {
        PropertyValue::Float(value) if !value.is_finite() => {
            Err(GraphError::Invalid("float property must be finite".into()))
        }
        PropertyValue::String(value) if value.len() > 1_048_576 => {
            Err(GraphError::Invalid("string property exceeds 1 MiB".into()))
        }
        PropertyValue::Bytes(value) if value.len() > 1_048_576 => {
            Err(GraphError::Invalid("byte property exceeds 1 MiB".into()))
        }
        _ => Ok(()),
    }
}
