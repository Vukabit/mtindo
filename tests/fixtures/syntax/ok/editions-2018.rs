// edition: 2018
async fn f() {
    let d: Box<dyn Trait> = g().await;
    let r#try = 1;
}
