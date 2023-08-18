use aws_sdk_s3::primitives::ByteStream;

pub struct S3Manager {
    client: aws_sdk_s3::Client,
}

impl S3Manager {
    pub fn new(config: &aws_config::SdkConfig) -> Self {
        Self {
            client: aws_sdk_s3::Client::new(config),
        }
    }

    pub async fn put_object(
        &self,
        bucket: &str,
        filename: &str,
        byte_stream: ByteStream,
    ) -> anyhow::Result<()> {
        self.client
            .put_object()
            .bucket(bucket)
            .key(filename)
            .body(byte_stream)
            .send()
            .await?;
        Ok(())
    }
}
