cfg_select! {
    feature = "shuttle" => { use shuttle::thread; }
    _ => { use std::thread; }
}

#[test]
fn race_insert() {
    check_dfs(|| {
        let map = arctic::concurrent::Map::<u64, u64, arctic::concurrent::smr::NoOp>::new();

        thread::scope(|scope| {
            let a = scope.spawn(|| {
                map.insert(5, 3)
                    .map(|value| *value)
                    .map_err(|(value, _)| *value)
            });
            let b = map
                .insert(5, 1)
                .map(|value| *value)
                .map_err(|(value, _)| *value);

            let a = a.join().unwrap();
            match (a, b) {
                (Ok(3), Err(3)) | (Err(1), Ok(1)) => (),
                _ => panic!("Impossible outcome: a={:?}, b={:x?}", a, b),
            }
        });
    });
}

fn check_dfs<F>(run: F)
where
    F: Fn() + Send + Sync + 'static,
{
    cfg_select! {
        feature = "shuttle" => { shuttle::check_dfs(run, None); }
        _ => {
            for _ in 0..1000 {
                run();
            }
        }
    }
}
