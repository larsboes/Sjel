use interior::model::{Model, ModelError};

pub fn load_model(flat: &str) -> Result<Model, ModelError> {
    Model::load_with_catalogue(flat, || {
        let store = sjel_inventory::store::Store::open(&sjel_config::database_path())?;
        store.catalogue()
    })
}
