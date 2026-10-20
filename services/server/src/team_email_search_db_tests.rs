#[tokio::test]
async fn team_email_search_uses_current_v2_memberships_and_admin_pagination() {
    let _guard = SERVER_TEST_LOCK.lock().await;
    let (database, pool, app, _storage, state) = test_application().await;
    let admin = fixture(&app, &database, "admin", Some(GlobalRole::Admin)).await;
    let leader = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
    let writer = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
    let mentor = fixture(&app, &database, "professor", Some(GlobalRole::Mentor)).await;
    let replacement = fixture(&app, &database, "professor", Some(GlobalRole::Mentor)).await;
    fixture_default_template(&state, &pool, &admin).await;
    let leader_id = test_user_id(&pool, &leader.email).await;
    let writer_id = test_user_id(&pool, &writer.email).await;
    let mentor_id = test_user_id(&pool, &mentor.email).await;
    let replacement_id = test_user_id(&pool, &replacement.email).await;
    let prefix = format!("Email search {}", uuid::Uuid::new_v4());
    let mut ids = Vec::new();
    for index in 0..3 {
        let response = request(&app, Method::POST, "/api/admin/v2/paper-teams", Some(&admin.cookie),
            &serde_json::json!({"name":format!("{prefix} {index}"),"writer_ids":[leader_id,writer_id],"leader_writer_id":leader_id,"mentor_ids":[mentor_id],"use_front_matter_default":false}).to_string(), Some("application/json")).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        ids.push(test_json(response).await["team"]["id"].clone());
    }
    let query = "/api/admin/v2/paper-teams/query";
    for cookie in [
        None,
        Some(leader.cookie.as_str()),
        Some(mentor.cookie.as_str()),
    ] {
        let response = get(&app, &format!("{query}?search={}", mentor.email), cookie).await;
        assert!(matches!(
            response.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ));
    }
    for search in [
        format!("search={}", mentor.email),
        format!("search=%20{}%20", mentor.email.to_uppercase()),
        format!("search={}", mentor.email.split('@').next().unwrap()),
        format!("search={}", leader.email),
        format!("mentor_email=%20{}%20", mentor.email.to_uppercase()),
        format!("leader_email={}", leader.email),
        format!("leader_email={}", leader.email.split('@').next().unwrap()),
        format!("search={}", prefix.replace(' ', "%20")),
        format!("mentor_user_id={mentor_id}&leader_user_id={leader_id}"),
    ] {
        let first = test_json(
            get(
                &app,
                &format!("{query}?{search}&limit=2&page=1"),
                Some(&admin.cookie),
            )
            .await,
        )
        .await;
        assert_eq!(first["total"], 3, "{search}: {first}");
        assert_eq!(first["items"].as_array().unwrap().len(), 2);
        assert_eq!(first["has_more"], true);
        let second = test_json(
            get(
                &app,
                &format!("{query}?{search}&limit=2&page=2"),
                Some(&admin.cookie),
            )
            .await,
        )
        .await;
        assert_eq!(second["total"], 3);
        assert_eq!(second["items"].as_array().unwrap().len(), 1);
        assert_eq!(second["has_more"], false);
        assert!(
            !first["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["id"] == second["items"][0]["id"])
        );
    }
    for search in [
        &writer.email,
        &replacement.email,
        "nobody@example.invalid",
        "%25",
        "%5F",
    ] {
        let page = test_json(
            get(
                &app,
                &format!("{query}?search={search}"),
                Some(&admin.cookie),
            )
            .await,
        )
        .await;
        assert_eq!(
            page["total"], 0,
            "non-Leader Writers, unassigned accounts and literal wildcard characters must not match: {page}"
        );
    }
    let response = request(&app, Method::PUT, &format!("/api/admin/v2/paper-teams/{}", ids[0].as_str().unwrap()), Some(&admin.cookie),
        &serde_json::json!({"name":format!("{prefix} changed"),"writer_ids":[leader_id,writer_id],"leader_writer_id":writer_id,"mentor_ids":[replacement_id]}).to_string(), Some("application/json")).await;
    assert_eq!(response.status(), StatusCode::OK);
    for (email, total) in [
        (&leader.email, 2),
        (&mentor.email, 2),
        (&writer.email, 1),
        (&replacement.email, 1),
    ] {
        let page = test_json(
            get(
                &app,
                &format!("{query}?search={email}"),
                Some(&admin.cookie),
            )
            .await,
        )
        .await;
        assert_eq!(page["total"], total);
        if total == 1 {
            assert_eq!(page["items"][0]["id"], ids[0]);
        }
    }
    let filtered = test_json(
        get(
            &app,
            &format!("{query}?mentor_email={}&status=frozen", mentor.email),
            Some(&admin.cookie),
        )
        .await,
    )
    .await;
    assert_eq!(filtered["total"], 0);
    let unresolved = test_json(
        get(
            &app,
            &format!("{query}?mentor_email={}&unresolved=true", mentor.email),
            Some(&admin.cookie),
        )
        .await,
    )
    .await;
    assert_eq!(unresolved["total"], 0);
    pool.close().await;
    database.close().await;
}
