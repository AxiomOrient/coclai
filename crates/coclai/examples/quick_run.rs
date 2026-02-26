use coclai::quick_run;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::var("COCLAI_CWD").unwrap_or_else(|_| {
        std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    });
    let prompt = std::env::var("COCLAI_PROMPT")
        .unwrap_or_else(|_| "현재 작업 디렉터리를 요약해줘".to_owned());

    let out = quick_run(cwd, prompt).await?;
    println!("{}", out.assistant_text);
    Ok(())
}
