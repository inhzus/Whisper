use std::time::Duration;

use anyhow::Context;
use aws_config::SdkConfig;
use aws_sdk_transcribe::types::{Media, TranscriptionJob, TranscriptionJobStatus};

pub struct Transcriber {
    client: aws_sdk_transcribe::Client,
}

impl Transcriber {
    pub fn new(config: &SdkConfig) -> Self {
        Self {
            client: aws_sdk_transcribe::Client::new(config),
        }
    }

    async fn wait(&self, job_name: &str) -> anyhow::Result<TranscriptionJob> {
        let mut snooze = Duration::from_millis(100);
        loop {
            let resp = self
                .client
                .get_transcription_job()
                .transcription_job_name(job_name)
                .send()
                .await
                .context("get transcription job")?;
            let job = resp
                .transcription_job()
                .context("retrieve transcription job")?;
            let status = job
                .transcription_job_status()
                .context("retrieve job status")?
                .clone();
            if status == TranscriptionJobStatus::Completed {
                return Ok(job.clone());
            }
            if status == TranscriptionJobStatus::Failed {
                return Err(anyhow::anyhow!("job failed"));
            }
            tokio::time::sleep(snooze).await;
            snooze = std::cmp::max(snooze * 2, Duration::from_secs(60));
        }
    }

    pub async fn run(&self, bucket: &str, filename: &str) -> anyhow::Result<String> {
        self.client
            .start_transcription_job()
            .transcription_job_name(filename)
            .media(
                Media::builder()
                    .media_file_uri(format!("s3://{bucket}/{filename}"))
                    .build(),
            )
            .send()
            .await
            .context("start transcription job")?;
        let job = self.wait(filename).await?;
        let transcript = job.transcript().context("retrieve job transcript")?;
        let uri = transcript
            .transcript_file_uri()
            .context("retrieve transcript uri")?;
        let json_body = reqwest::get(uri).await?.text().await?;
        Ok(json_body)
    }
}
