use std::{fs::File, path::Path};

use anyhow::{Result, Context};

use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
/// Photon record from CSV/HDF5 file.
///
/// Contains position, direction, and properties for each photon:
/// - Position (pos_x, pos_y, pos_z)
/// - Direction (dir_x, dir_y, dir_z)
/// - Physical properties (wavelength, power, tof)
/// - MC simulation properties (weight)
/// - UID reference (uid, encoded as u64)
pub struct Record {
    pub pos_x:      f64,
    pub pos_y:      f64,
    pub pos_z:      f64,
    pub dir_x:      f64,
    pub dir_y:      f64,
    pub dir_z:      f64,
    pub wavelength: f64,
    pub power:      f64,
    pub weight:     f64,
    pub tof:        f64,
    #[serde(
        serialize_with = "array_bytes::ser_hexify",
        deserialize_with = "array_bytes::de_dehexify"
    )]
    pub uid:        u64,
}

pub fn read_signals_csv(path: &Path) -> std::io::Result<Vec<Record>> {
    let file = File::open(path)?;
    let mut reader = csv::Reader::from_reader(file);
    let mut records = Vec::new();

    for result in reader.deserialize() {
        let record: Record = result?;
        records.push(record);
    }

    Ok(records)
}

pub fn read_signals_hdf5(path: &std::path::Path) -> hdf5_metno::Result<Vec<Record>> {
    let file = hdf5_metno::File::open(path)?;
    let pos_x: Vec<f64>      = file.dataset("pos_x")?.read_1d()?.to_vec();
    let pos_y: Vec<f64>      = file.dataset("pos_y")?.read_1d()?.to_vec();
    let pos_z: Vec<f64>      = file.dataset("pos_z")?.read_1d()?.to_vec();
    let dir_x: Vec<f64>      = file.dataset("dir_x")?.read_1d()?.to_vec();
    let dir_y: Vec<f64>      = file.dataset("dir_y")?.read_1d()?.to_vec();
    let dir_z: Vec<f64>      = file.dataset("dir_z")?.read_1d()?.to_vec();
    let wavelength: Vec<f64> = file.dataset("wavelength")?.read_1d()?.to_vec();
    let power: Vec<f64>      = file.dataset("power")?.read_1d()?.to_vec();
    let weight: Vec<f64>     = file.dataset("weight")?.read_1d()?.to_vec();
    let tof: Vec<f64>        = file.dataset("tof")?.read_1d()?.to_vec();
    let uid: Vec<u64>        = file.dataset("uid")?.read_1d()?.to_vec();

    let n = pos_x.len();
    assert!(n == pos_y.len() && n == pos_z.len());
    assert!(n == dir_x.len() && n == dir_y.len() && n == dir_z.len());
    assert!(n == wavelength.len() && n == power.len());
    assert!(n == weight.len() && n == tof.len() && n == uid.len());

    let records = (0..n).map(|i| {
        Record {
            pos_x: pos_x[i],
            pos_y: pos_y[i],
            pos_z: pos_z[i],
            dir_x: dir_x[i],
            dir_y: dir_y[i],
            dir_z: dir_z[i],
            wavelength: wavelength[i],
            power: power[i],
            weight: weight[i],
            tof: tof[i],
            uid: uid[i],
        }
    }).collect::<Vec<_>>();

    Ok(records)
}

pub fn read_signals(signals_path: &Path) -> Result<Vec<Record>> {
    let extension = signals_path.extension().and_then(|ext| ext.to_str()).ok_or_else(|| {
        anyhow::anyhow!(
            "Signals path must have a valid extension (e.g., .csv or .hdf5): {:?}",
            signals_path
        )
    })?;
    match extension {
        "csv" => read_signals_csv(signals_path)
            .context("Failed to read signals from CSV file"),
        "hdf5" | "h5" => read_signals_hdf5(signals_path)
            .context("Failed to read signals from HDF5 file"),
        _ => Err(anyhow::anyhow!(
            "Unsupported signals file extension: {}",
            extension
        )),
    }
}

pub fn write_signals(signals: Vec<&Record>, output_path: &Path) -> Result<()> {
    let extension = output_path.extension().and_then(|ext| ext.to_str()).ok_or_else(|| {
        anyhow::anyhow!(
            "Output path must have a valid extension (e.g., .csv): {:?}",
            output_path
        )
    })?;

    match extension {
        "csv" => write_signals_csv(signals, output_path)?,
        "hdf5" | "h5" => write_signals_hdf5(signals, output_path)?,
        _ => return Err(anyhow::anyhow!(
            "Unsupported output file extension: {}",
            extension
        )),
    };

    Ok(())
}

fn write_signals_csv(signals: Vec<&Record>, output_path: &Path) -> Result<()> {
    let mut wtr = csv::Writer::from_path(output_path).context("Failed to create CSV writer")?;
    for record in signals {
        wtr.serialize(record).context("Failed to write CSV record")?;
    }
    wtr.flush().context("Failed to flush CSV writer")?;
    Ok(())
}

fn write_signals_hdf5(signals: Vec<&Record>, output_path: &Path) -> Result<()> {
    let file = hdf5_metno::File::create(output_path).context("Failed to create HDF5 file")?;
    // Create datasets for each field in Record
    let pos_x: Vec<f64> = signals.iter().map(|r| r.pos_x).collect();
    let pos_y: Vec<f64> = signals.iter().map(|r| r.pos_y).collect();
    let pos_z: Vec<f64> = signals.iter().map(|r| r.pos_z).collect();
    let dir_x: Vec<f64> = signals.iter().map(|r| r.dir_x).collect();
    let dir_y: Vec<f64> = signals.iter().map(|r| r.dir_y).collect();
    let dir_z: Vec<f64> = signals.iter().map(|r| r.dir_z).collect();
    let wavelength: Vec<f64> = signals.iter().map(|r| r.wavelength).collect();
    let power: Vec<f64> = signals.iter().map(|r| r.power).collect();
    let weight: Vec<f64> = signals.iter().map(|r| r.weight).collect();
    let tof: Vec<f64> = signals.iter().map(|r| r.tof).collect();
    let uid: Vec<u64> = signals.iter().map(|r| r.uid).collect();
    file.new_dataset_builder().with_data(&pos_x).create("pos_x")?;
    file.new_dataset_builder().with_data(&pos_y).create("pos_y")?;
    file.new_dataset_builder().with_data(&pos_z).create("pos_z")?;
    file.new_dataset_builder().with_data(&dir_x).create("dir_x")?;
    file.new_dataset_builder().with_data(&dir_y).create("dir_y")?;
    file.new_dataset_builder().with_data(&dir_z).create("dir_z")?;
    file.new_dataset_builder().with_data(&wavelength).create("wavelength")?;
    file.new_dataset_builder().with_data(&power).create("power")?;
    file.new_dataset_builder().with_data(&weight).create("weight")?;
    file.new_dataset_builder().with_data(&tof).create("tof")?;
    file.new_dataset_builder().with_data(&uid).create("uid")?;

    Ok(())
}
