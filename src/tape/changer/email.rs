use anyhow::Error;

use proxmox::tools::email::sendmail;

use super::MediaChange;

/// Send email to a person to request a manual media change
pub struct ChangeMediaEmail {
    drive: String,
    to: String,
}

impl ChangeMediaEmail {

    pub fn new(drive: &str, to: &str) -> Self {
        Self {
            drive: String::from(drive),
            to: String::from(to),
        }
    }
}

impl MediaChange for ChangeMediaEmail {

    fn load_media(&mut self, changer_id: &str) -> Result<(), Error> {

        let subject = format!("Load Media '{}' request for drive '{}'", changer_id, self.drive);

        let mut text = String::new();

        text.push_str("Please insert the requested media into the backup drive.\n\n");

        text.push_str(&format!("Drive: {}\n", self.drive));
        text.push_str(&format!("Media: {}\n", changer_id));

        sendmail(
            &[&self.to],
            &subject,
            Some(&text),
            None,
            None,
            None,
        )?;

        Ok(())
    }

    fn unload_media(&mut self) -> Result<(), Error> {
        /* ignore ? */
        Ok(())
    }

    fn list_media_changer_ids(&self) -> Result<Vec<String>, Error> {
        Ok(Vec::new())
    }

}
