use criterion::{black_box, criterion_group, criterion_main, BenchmarkGroup, Criterion};

fn liquidate(_n: u64) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    runtime.block_on(async move {
        // solend_liquidator::client::run_liquidator_iter(String::from(
        //     "../solend-liquidator/private/liquidator_main.json",
        // ))
        // .await;

        // let obligation_path = String::from("./fixtures/calculate_refreshed_obligation_9fGaP5fHsCAt7J1PK97kbXTPYjG6UiDGBw1VT6FSCnr3");

        // let data = tokio::fs::read(obligation_path).await.unwrap();
        // let obligation_fixture: client::fixtures::CalculateRefreshedObligationFixture =
        //     serde_json::from_slice(&data).unwrap();

        // // println!("obligation_fixture: {:?}", obligation_fixture);

        // let arguments = obligation_fixture.decode();
        // let refreshed_obligation = calculate_refreshed_obligation(
        //     &Enhanced {
        //         inner: arguments.0,
        //         pubkey: Pubkey::from_str("9fGaP5fHsCAt7J1PK97kbXTPYjG6UiDGBw1VT6FSCnr3").unwrap(),
        //     },
        //     &arguments.1,
        //     &arguments.2,
        // )
        // .await
        // .unwrap();

        // let decimals = 1e8 as f64;
        // println!("refreshed_obligation: {:?}", refreshed_obligation);
        // println!("refreshed_obligation.borrowed_value: {:?}", refreshed_obligation.borrowed_value.as_u128() as f64 / decimals);
        // println!("refreshed_obligation.liquidation_limit: {:?}", refreshed_obligation.unhealthy_borrow_value.as_u128() as f64 / decimals);

        // // let borrowed_value_real = 749209978243u128;
        // // let unhealthy_borrow_real = 849493781460u128;
        // // println!("unhealthy_borrow_real: {:?}", unhealthy_borrow_real);

        // // assert_eq!(refreshed_obligation.borrowed_value.as_u128(), borrowed_value_real);
        // // assert_eq!(refreshed_obligation.unhealthy_borrow_value.as_u128(), unhealthy_borrow_real);
    })
}

fn criterion_benchmark(c: &mut Criterion) {
    // c.bench_function("liquidate iter", |b| b.iter(|| liquidate(black_box(1))));

    // let b = c.sample_size(10);
    c.bench_function(
        "liquidate iter",
        BenchmarkGroup::new("routine_1", |b| b.iter(|| liquidate(1))).sample_size(10),
    );

    //     c.bench(
    // /         "routines",
    // /         Benchmark::new("routine_1", |b| b.iter(|| routine_1()))
    // /             .with_function("routine_2", |b| b.iter(|| routine_2()))
    // /             .sample_size(50)
    // /     );
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
