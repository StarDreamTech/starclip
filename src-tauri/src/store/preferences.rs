use super::*;

impl Store {
    pub fn settings(&self) -> Result<Settings> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM preferences WHERE key='settings'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        match json {
            Some(json) => {
                let settings: Settings =
                    serde_json::from_str(&json).map_err(|_| "偏好设置损坏，请从备份恢复。")?;
                settings.validate()?;
                Ok(settings)
            }
            None => Ok(Settings::default()),
        }
    }

    pub fn window_startup_preferences(&self) -> Result<WindowStartupPreferences> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM preferences WHERE key='window_startup_v1'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        match json {
            Some(json) => serde_json::from_str(&json)
                .map_err(|_| "启动与菜单栏偏好损坏，请从备份恢复。".into()),
            None => Ok(WindowStartupPreferences::default()),
        }
    }

    pub fn save_window_startup_preferences(
        &self,
        preferences: &WindowStartupPreferences,
    ) -> Result<()> {
        let json = serde_json::to_string(preferences).map_err(|error| error.to_string())?;
        self.conn
            .execute(
                "INSERT INTO preferences(key,value) VALUES('window_startup_v1',?1)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [json],
            )
            .map_err(sql_error)?;
        Ok(())
    }

    pub fn telemetry_preferences(&self) -> Result<TelemetryPreferences> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM preferences WHERE key='telemetry_preferences_v1'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        match json {
            Some(json) => {
                let preferences: TelemetryPreferences = serde_json::from_str(&json)
                    .map_err(|_| "统计偏好损坏，请重新设置。".to_string())?;
                if preferences.revision > MAX_REVISION {
                    return Err("统计偏好损坏，请重新设置。".into());
                }
                Ok(preferences)
            }
            None => Ok(TelemetryPreferences::default()),
        }
    }

    pub fn set_telemetry_enabled(&self, enabled: bool) -> Result<TelemetryPreferences> {
        // Only an explicit local choice can replace damaged preferences. This
        // never touches ordinary settings or retention.
        let mut preferences = TelemetryPreferences {
            revision: self
                .telemetry_preferences()
                .map(|value| value.revision)
                .unwrap_or(0),
            enabled,
        };
        preferences.revision = preferences
            .revision
            .checked_add(1)
            .filter(|revision| *revision <= MAX_REVISION)
            .ok_or("统计偏好修订已达上限。")?;
        let json = serde_json::to_string(&preferences).map_err(|error| error.to_string())?;
        self.conn.execute(
            "INSERT INTO preferences(key,value) VALUES('telemetry_preferences_v1',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [json],
        ).map_err(sql_error)?;
        Ok(preferences)
    }

    pub fn update_preferences(&self) -> Result<UpdatePreferences> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM preferences WHERE key='update_preferences_v1'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        match json {
            Some(json) => {
                serde_json::from_str(&json).map_err(|_| "更新偏好损坏，请重新启动星笺。".into())
            }
            None => Ok(UpdatePreferences::default()),
        }
    }

    pub(super) fn save_update_preferences(
        &self,
        preferences: &mut UpdatePreferences,
    ) -> Result<()> {
        preferences.revision = preferences
            .revision
            .checked_add(1)
            .filter(|revision| *revision <= MAX_REVISION)
            .ok_or("更新偏好修订已达上限。")?;
        let json = serde_json::to_string(preferences).map_err(|error| error.to_string())?;
        self.conn
            .execute(
                "INSERT INTO preferences(key,value) VALUES('update_preferences_v1',?1)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [json],
            )
            .map_err(sql_error)?;
        Ok(())
    }

    pub fn set_auto_update_check(&self, enabled: bool) -> Result<UpdatePreferences> {
        let mut preferences = self.update_preferences()?;
        if preferences.auto_check != enabled {
            preferences.auto_check = enabled;
            self.save_update_preferences(&mut preferences)?;
        }
        Ok(preferences)
    }

    pub fn skip_update_version(&self, version: &str) -> Result<UpdatePreferences> {
        let mut preferences = self.update_preferences()?;
        preferences.skipped_version = Some(version.to_owned());
        self.save_update_preferences(&mut preferences)?;
        Ok(preferences)
    }

    // The caller holds the storage/store/updater locks in that order. Persist
    // before requesting; both a failed request and a restart consume the interval.
    pub fn record_update_check(
        &self,
        now: u64,
        automatic: bool,
    ) -> Result<(UpdatePreferences, bool)> {
        let mut preferences = self.update_preferences()?;
        if automatic {
            if !preferences.auto_check {
                return Ok((preferences, false));
            }
            if let Some(last) = preferences.last_check_at {
                if now < last {
                    preferences.last_check_at = Some(now);
                    self.save_update_preferences(&mut preferences)?;
                    return Ok((preferences, false));
                }
                if now - last < AUTO_UPDATE_INTERVAL_MS {
                    return Ok((preferences, false));
                }
            }
        }
        preferences.last_check_at = Some(now);
        self.save_update_preferences(&mut preferences)?;
        Ok((preferences, true))
    }

    pub fn onboarding_completed(&self) -> Result<bool> {
        self.conn
            .query_row(
                "SELECT COALESCE((SELECT value='true' FROM preferences WHERE key='onboarding_v1'), 1)",
                [],
                |row| row.get(0),
            )
            .map_err(sql_error)
    }

    pub fn complete_onboarding(&self) -> Result<()> {
        // This marker must not run save_settings, pruning, or clipboard transitions.
        self.conn
            .execute(
                "INSERT INTO preferences(key,value) VALUES('onboarding_v1','true')
            ON CONFLICT(key) DO UPDATE SET value='true'",
                [],
            )
            .map_err(sql_error)?;
        Ok(())
    }

    pub fn appearance(&self) -> Result<Appearance> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM preferences WHERE key='appearance_v1'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        let value = match json {
            Some(json) => serde_json::from_str::<Appearance>(&json)
                .map_err(|_| "自定义外观数据损坏，未修改已保存的数据。")?,
            None => Appearance::default(),
        };
        value
            .validate()
            .map_err(|_| "自定义外观数据损坏，未修改已保存的数据。")?;
        Ok(value)
    }

    pub(super) fn change_appearance(
        &self,
        change: impl FnOnce(&Appearance) -> Result<Appearance>,
    ) -> Result<Appearance> {
        // Appearance updates never save ordinary settings, run retention or award growth.
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        let old = self.appearance()?;
        let next = change(&old)?;
        next.validate()?;
        if next != old {
            let json = serde_json::to_string(&next).map_err(|_| "无法保存自定义外观。")?;
            tx.execute("INSERT INTO preferences(key,value) VALUES('appearance_v1',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [json]).map_err(sql_error)?;
        }
        tx.commit().map_err(sql_error)?;
        Ok(next)
    }

    pub fn save_appearance(&self, patch: &serde_json::Value) -> Result<Appearance> {
        self.change_appearance(|value| value.patched(patch))
    }

    pub fn import_appearance_sticker(&self, png_base64: &str, name: &str) -> Result<Appearance> {
        let sticker = appearance::prepare_sticker(png_base64, name)?;
        self.change_appearance(|value| Ok(value.with_sticker(sticker)))
    }

    pub fn remove_appearance_sticker(&self) -> Result<Appearance> {
        self.change_appearance(|value| Ok(value.without_sticker()))
    }

    pub fn starpet_preferences(&self) -> Result<StarpetPreferences> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM preferences WHERE key='starpet_v1'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        match json {
            Some(json) => {
                let mut preferences: StarpetPreferences = serde_json::from_str(&json)
                    .map_err(|_| "星伴偏好损坏，未更改已保存的数据。")?;
                preferences.validate()?;
                preferences.color.make_ascii_uppercase();
                Ok(preferences)
            }
            None => Ok(StarpetPreferences::default()),
        }
    }

    pub fn starpet_growth(&self) -> Result<GrowthState> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM preferences WHERE key='starpet_growth_v1'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        let state = match json {
            Some(json) => serde_json::from_str::<GrowthState>(&json)
                .map_err(|_| "星伴成长存档损坏，未修改已保存的数据。")?,
            None => GrowthState::default(),
        };
        state.validate()?;
        Ok(state)
    }

    pub fn change_starpet_growth(&self, change: GrowthChange<'_>) -> Result<GrowthState> {
        let today: String = self
            .conn
            .query_row("SELECT strftime('%Y-%m-%d','now','localtime')", [], |row| {
                row.get(0)
            })
            .map_err(sql_error)?;
        self.change_starpet_growth_on(change, &today)
    }

    pub(super) fn change_starpet_growth_on(
        &self,
        change: GrowthChange<'_>,
        today: &str,
    ) -> Result<GrowthState> {
        // One transaction contains read, validation and write; unrelated preferences and clips are untouched.
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        let old = self.starpet_growth()?;
        let mut state = old.clone();
        change.apply(&mut state, today)?;
        if state != old {
            state.revision = old
                .revision
                .checked_add(1)
                .filter(|r| *r <= MAX_REVISION)
                .ok_or("成长存档修订次数超限，未修改数据。")?;
            let json = serde_json::to_string(&state).map_err(|_| "无法保存星伴成长数据。")?;
            tx.execute("INSERT INTO preferences(key,value) VALUES('starpet_growth_v1',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [json]).map_err(sql_error)?;
        }
        tx.commit().map_err(sql_error)?;
        Ok(state)
    }

    pub fn save_starpet_preferences(&self, preferences: &StarpetPreferences) -> Result<()> {
        // Keep pet interactions independent of retention, clipboard capture and ordinary settings.
        preferences.validate()?;
        let mut preferences = preferences.clone();
        preferences.color.make_ascii_uppercase();
        let json = serde_json::to_string(&preferences).map_err(|e| e.to_string())?;
        self.conn
            .execute(
                "INSERT INTO preferences(key,value) VALUES('starpet_v1',?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [json],
            )
            .map_err(sql_error)?;
        Ok(())
    }

    pub fn set_theme(&self, theme: &str) -> Result<Settings> {
        if !["dark", "light", "white", "forest", "rose", "system"].contains(&theme) {
            return Err("不支持的应用主题。".into());
        }
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        let stored: Option<String> = tx
            .query_row(
                "SELECT value FROM preferences WHERE key='settings'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        let mut json: serde_json::Value = match stored {
            Some(json) => serde_json::from_str(&json).map_err(|_| "偏好设置损坏，主题未保存。")?,
            None => serde_json::to_value(Settings::default()).map_err(|e| e.to_string())?,
        };
        json.as_object_mut()
            .ok_or("偏好设置损坏，主题未保存。")?
            .insert("theme".into(), theme.into());
        let settings: Settings =
            serde_json::from_value(json.clone()).map_err(|_| "偏好设置损坏，主题未保存。")?;
        settings.validate()?;
        // Preserve every other stored preference, including older missing fields and future keys.
        tx.execute("INSERT INTO preferences(key,value) VALUES('settings',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [json.to_string()]).map_err(sql_error)?;
        tx.commit().map_err(sql_error)?;
        Ok(settings)
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        self.save_settings_with_pause_guard(settings, None)
            .map(|_| ())
    }

    pub fn save_settings_with_pause_guard(
        &self,
        settings: &Settings,
        expected_pause_revision: Option<u64>,
    ) -> Result<Settings> {
        settings.validate()?;
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        let mut settings = settings.clone();
        // Read and write on this transaction. The version detects even an
        // unseen pause -> resume -> pause round trip while a draft is open.
        let current = self.settings()?;
        if expected_pause_revision.is_some_and(|expected| expected != current.pause_revision) {
            settings.paused = current.paused;
        }
        // The store owns this version; never persist a caller-supplied value.
        settings.pause_revision = if settings.paused != current.paused {
            current
                .pause_revision
                .checked_add(1)
                .ok_or("采集设置版本已达上限，偏好设置未保存。")?
        } else {
            current.pause_revision
        };
        let json = serde_json::to_string(&settings).map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO preferences(key,value) VALUES('settings',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [json]).map_err(sql_error)?;
        Self::prune_with_settings(&tx, &settings, now())?;
        tx.commit().map_err(sql_error)?;
        Ok(settings)
    }
}
