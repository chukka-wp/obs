/**
 * chukka-obs composite overlay — connects to /display WebSocket,
 * renders all overlay regions from DisplayState.
 *
 * This overlay is a dumb renderer. It never decides what to show.
 * All visibility and timing decisions come from chukka-obs (Rust).
 */

(function () {
    'use strict';

    // -----------------------------------------------------------------------
    // State
    // -----------------------------------------------------------------------

    let gameState = null;
    let displayState = null;
    let matchConfig = null;
    let ws = null;

    // -----------------------------------------------------------------------
    // WebSocket connection
    // -----------------------------------------------------------------------

    function connect() {
        const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
        const url = `${protocol}//${location.host}/display`;

        ws = new WebSocket(url);

        ws.onopen = () => {
            console.log('[chukka-obs] Connected to /display');
            fetchConfig();
        };

        ws.onmessage = (event) => {
            try {
                const data = JSON.parse(event.data);
                gameState = data.game_state;
                displayState = data.display;
                render();
            } catch (e) {
                console.error('[chukka-obs] Parse error:', e);
            }
        };

        ws.onclose = () => {
            console.log('[chukka-obs] Disconnected — reconnecting in 2s');
            setTimeout(connect, 2000);
        };

        ws.onerror = () => {
            ws.close();
        };
    }

    function fetchConfig() {
        fetch('/config')
            .then(r => r.ok ? r.json() : null)
            .then(data => {
                if (data) {
                    matchConfig = data;
                    applyBranding();
                    render();
                }
            })
            .catch(() => {});
    }

    // -----------------------------------------------------------------------
    // Branding
    // -----------------------------------------------------------------------

    function applyBranding() {
        if (!matchConfig) return;

        const root = document.documentElement;
        root.style.setProperty('--home-colour', matchConfig.home_team.cap_colour);
        root.style.setProperty('--away-colour', matchConfig.away_team.cap_colour);
        root.style.setProperty('--home-text', contrastText(matchConfig.home_team.cap_colour));
        root.style.setProperty('--away-text', contrastText(matchConfig.away_team.cap_colour));
    }

    function contrastText(hex) {
        if (!hex) return '#ffffff';
        const c = hex.replace('#', '');
        const r = parseInt(c.substr(0, 2), 16);
        const g = parseInt(c.substr(2, 2), 16);
        const b = parseInt(c.substr(4, 2), 16);
        const luminance = (0.299 * r + 0.587 * g + 0.114 * b) / 255;
        return luminance > 0.5 ? '#000000' : '#ffffff';
    }

    // -----------------------------------------------------------------------
    // Render
    // -----------------------------------------------------------------------

    function render() {
        if (!gameState || !displayState) return;

        renderScorebug();
        renderExclusions();
        renderGoalAnimation();
        renderFoulOut();
        renderQuarterSummary();
        renderPossessionClock();
        renderShootout();
        renderLowerThird();
    }

    function toggleRegion(id, visible) {
        const el = document.getElementById(id);
        if (!el) return;
        el.classList.toggle('visible', visible);
    }

    // -----------------------------------------------------------------------
    // Scorebug
    // -----------------------------------------------------------------------

    function renderScorebug() {
        toggleRegion('scorebug', displayState.scorebug.visible);
        if (!displayState.scorebug.visible) return;

        const gs = gameState;
        const homeName = matchConfig?.home_team?.short_name || 'HOME';
        const awayName = matchConfig?.away_team?.short_name || 'AWAY';

        setText('sb-home-name', homeName);
        setText('sb-away-name', awayName);
        setText('sb-home-score', gs.home_score);
        setText('sb-away-score', gs.away_score);
        setText('sb-period', formatPeriod(gs.current_period, gs.status));
        setText('sb-clock', formatClock(gs.period_clock_seconds));

        const homePoss = document.getElementById('sb-home-poss');
        const awayPoss = document.getElementById('sb-away-poss');
        if (homePoss) homePoss.classList.toggle('active', gs.possession === 'home');
        if (awayPoss) awayPoss.classList.toggle('active', gs.possession === 'away');
    }

    // -----------------------------------------------------------------------
    // Exclusions
    // -----------------------------------------------------------------------

    function renderExclusions() {
        toggleRegion('exclusions', displayState.exclusions.visible);
        if (!displayState.exclusions.visible) return;

        const container = document.getElementById('exc-slots');
        if (!container) return;

        const exclusions = gameState.active_exclusions || [];

        // Clear existing slots using safe DOM methods.
        while (container.firstChild) {
            container.removeChild(container.firstChild);
        }

        exclusions.forEach(exc => {
            const isHome = isHomeTeam(exc.team_id);
            const colour = isHome
                ? (matchConfig?.home_team?.cap_colour || '#003087')
                : (matchConfig?.away_team?.cap_colour || '#ffffff');
            const textColour = contrastText(colour);

            const slot = document.createElement('div');
            slot.className = 'exc-slot' + (exc.exclusion_type === 'violent_action' ? ' violent' : '');

            const capDiv = document.createElement('div');
            capDiv.className = 'exc-cap';
            capDiv.style.background = colour;
            capDiv.style.color = textColour;
            capDiv.textContent = exc.cap_number;

            const timerDiv = document.createElement('div');
            timerDiv.className = 'exc-timer';
            timerDiv.textContent = formatClock(exc.remaining_seconds);

            slot.appendChild(capDiv);
            slot.appendChild(timerDiv);
            container.appendChild(slot);
        });
    }

    // -----------------------------------------------------------------------
    // Goal animation
    // -----------------------------------------------------------------------

    function renderGoalAnimation() {
        const ga = displayState.goal_animation;
        toggleRegion('goal-animation', ga.visible);
        if (!ga.visible) return;

        const teamName = ga.scoring_team === 'home'
            ? (matchConfig?.home_team?.short_name || 'HOME')
            : (matchConfig?.away_team?.short_name || 'AWAY');

        setText('goal-team-name', teamName);
        setText('goal-score-display', `${ga.home_score ?? 0} \u2013 ${ga.away_score ?? 0}`);

        const capEl = document.getElementById('goal-cap-display');
        if (capEl) {
            capEl.textContent = ga.cap_number ? `#${ga.cap_number}` : '';
            capEl.style.display = ga.cap_number ? '' : 'none';
        }
    }

    // -----------------------------------------------------------------------
    // Foul-out
    // -----------------------------------------------------------------------

    function renderFoulOut() {
        const fo = displayState.foul_out;
        toggleRegion('foul-out', fo.visible);
        if (!fo.visible) return;

        setText('foulout-cap-display', fo.cap_number != null ? `#${fo.cap_number}` : '');
        setText('foulout-count', fo.foul_count != null ? `${fo.foul_count} personal fouls` : '');
    }

    // -----------------------------------------------------------------------
    // Quarter summary
    // -----------------------------------------------------------------------

    function renderQuarterSummary() {
        const qs = displayState.quarter_summary;
        toggleRegion('quarter-summary', qs.visible);
        if (!qs.visible) return;

        const period = qs.period_completed || (gameState.current_period - 1);
        setText('qs-period-display', `End of Q${period}`);
        setText('qs-score-display', `${qs.home_score ?? gameState.home_score} \u2013 ${qs.away_score ?? gameState.away_score}`);
    }

    // -----------------------------------------------------------------------
    // Possession clock
    // -----------------------------------------------------------------------

    function renderPossessionClock() {
        toggleRegion('possession-clock', displayState.possession_clock.visible);
        if (!displayState.possession_clock.visible) return;

        const secs = gameState.possession_clock_seconds ?? 0;
        const timeEl = document.getElementById('pc-time');
        if (timeEl) {
            timeEl.textContent = Math.ceil(secs);
            timeEl.classList.toggle('urgent', secs <= 5);
        }

        const mode = gameState.possession_clock_mode || 'standard';
        setText('pc-mode', mode === 'reduced' ? '18s' : '28s');
    }

    // -----------------------------------------------------------------------
    // Shootout
    // -----------------------------------------------------------------------

    function renderShootout() {
        toggleRegion('shootout', displayState.shootout.visible);
        if (!displayState.shootout.visible || !gameState.shootout_state) return;

        const so = gameState.shootout_state;
        const homeName = matchConfig?.home_team?.short_name || 'HOME';
        const awayName = matchConfig?.away_team?.short_name || 'AWAY';

        setText('so-home-name', homeName);
        setText('so-away-name', awayName);
        setText('so-home-score', so.home_score);
        setText('so-away-score', so.away_score);
        setText('so-round-display', `Round ${so.current_round}`);

        // Render shot indicators using safe DOM methods.
        const shotsEl = document.getElementById('so-shots-display');
        if (shotsEl && so.shots) {
            while (shotsEl.firstChild) {
                shotsEl.removeChild(shotsEl.firstChild);
            }

            so.shots.forEach(shot => {
                const div = document.createElement('div');
                const cls = shot.outcome === 'goal' ? 'goal' : shot.outcome;
                div.className = `so-shot ${cls}`;
                div.textContent = shot.outcome === 'goal' ? '\u2713' : '\u2717';
                shotsEl.appendChild(div);
            });
        }
    }

    // -----------------------------------------------------------------------
    // Lower third
    // -----------------------------------------------------------------------

    function renderLowerThird() {
        const lt = displayState.lower_third;
        toggleRegion('lower-third', lt.visible);
        if (!lt.visible) return;

        setText('lt-cap-display', lt.cap_number ?? '');
        setText('lt-name-display', lt.player_name ?? '');
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    function setText(id, text) {
        const el = document.getElementById(id);
        if (el) el.textContent = text;
    }

    function formatClock(seconds) {
        if (seconds == null) return '0:00';
        const s = Math.max(0, Math.ceil(seconds));
        const m = Math.floor(s / 60);
        const sec = s % 60;
        return `${m}:${sec.toString().padStart(2, '0')}`;
    }

    function isHomeTeam(teamId) {
        if (!teamId || !matchConfig) return true;

        const homeId = matchConfig.home_team?.id;
        const awayId = matchConfig.away_team?.id;

        if (homeId && teamId === homeId) return true;
        if (awayId && teamId === awayId) return false;

        return true;
    }

    function formatPeriod(period, status) {
        if (status === 'not_started') return 'PRE';
        if (status === 'completed') return 'FINAL';
        if (status === 'abandoned') return 'ABD';
        if (status === 'halftime') return 'HT';
        if (status === 'shootout') return 'SO';
        if (status === 'overtime') return `OT${period - 4}`;
        if (status === 'period_break') return `Q${Math.max(1, period - 1)} END`;
        if (period <= 4) return `Q${period}`;
        return `OT${period - 4}`;
    }

    // -----------------------------------------------------------------------
    // Init
    // -----------------------------------------------------------------------

    document.addEventListener('DOMContentLoaded', connect);
})();
