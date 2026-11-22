# ribozym — Progress Ledger

Plan: `C:/Users/Rog/Desktop/agent-sistem-planlama-v2.md` (v2.47). Gates:
`python scripts/validate_skeleton.py` (55 checks) + `cargo test` + `cargo clippy --all-targets` (0 warnings) + `cargo fmt --check`.

## Faz 0 — İskelet — DONE
schemas/ (11), config/ (agents.toml A1–A12 + 12 prompt + models.toml), `.ribozym/migrations/0001_init.sql`, examples/ fixtures, skeleton validator. 52/52 green.

## Faz 1 — Modeller + SQLite + olay zarfı + ledger — DONE
`src/models/` (spec, task, execution, report, event — extra-tolerant), `src/state.rs` (WAL + busy_timeout + migration runner), `src/config/`, `src/error.rs` (§16 taxonomy), `src/validate.rs` (structural + full jsonschema; `$id` stripped at load — relative ids break the resolver), CLI `config validate` / `schema validate [--full]`.

## Faz 2 — Çıkarım + ortam katmanı + MockLLM + repair — DONE
- `src/inference/`: `InferenceProvider` trait, `CloudProvider` (OpenRouter-compatible, reqwest), `LocalProvider` (llama-server), `MockProvider` (scripted steps; wiremock yerine — her senaryo veri olarak).
- `src/inference/env.rs`: `EnvLayer` — pre-clamp bütçe, full-jitter E0 backoff (deterministik xorshift), estimate→actual mutabakatı.
- `src/repair.rs`: **SchemaGuard** — JSON ayıklama + tam şema doğrulama + TEK seferlik düzeltme turu (§14); ikinci hata → `MODEL_SCHEMA_INVALID`. Onarım turu sabit öneki bayt-bayta korur (§10.4 caching). Çıkış kriteri 4 testle kanıtlı.

## Faz 3 — Agent arayüzü + ToolGateway — DONE (bu oturum)
- `src/config/project.rs`: `[toolchains.*]` → `ToolchainSpec` (allowed, fs_read/write, env_scrub, timeout_s, output_cap_kb).
- `src/gateway.rs`: **ToolGateway** — normalize → **tam-argv eşleşmeli allowlist** (prefix değil: `cargo check --config ...` ✘) → scrubbed env (`*_KEY/*_TOKEN/*_SECRET` silinir) → `CommandRunner` (spawn'sız timeout + output cap, UTF-8 sınırında kırpma) → `preflight()` = TOOLCHAIN_MISSING (§12.4). Ret kararı runner'a ASLA ulaşmaz.
- `src/agent.rs`: `Agent` trait + `AgentContext` + `TaskInput` — LLM yalnız `SchemaGuard`'dan, araç yalnız `ToolGateway`'den.
- Çıkış kriterleri: argv-fuzz (2000 deterministik girdi, exact-vector admission), allowlist ret, env scrub, preflight — hepsi `src/gateway.rs` testlerinde.
- Stats: **17 unit + 12 integration = 29 test**, clippy 0 warning, fmt clean, skeleton 52/52.

## Faz 3+ — Grunt-work routing (Q1 kararı) — DONE (bu oturum)
- `src/select.rs`: **ayak işleri tespiti + yerel yönlendirme.**
  - `classify`: config-bazlı üyelik kontrolü (LLM karar vermez, P3); liste
    `models.toml [local].eligible_task_types` — boilerplate, template_code,
    test_skeleton, classification, schema_repair, summarization.
  - `vram_admits`: §10.1 kuralı — model yükü (Qwen2.5-Coder-7B Q4_K_M
    `C:\Users\Rog\qwen_model\` doğrulandı) + KV (~60 KiB/token sabit tahmin)
    boş VRAM'i aşarsa görev **sessizce buluta**.
  - `LocalFirstRouting` (InferenceProvider): grunt+kabul → local; local hata →
    cloud'a tek düşüş, eskalasyon yok; `provider` alanı gerçek motoru yazar.
  - VRAM probe'u çağrı-başına okunur: orta-kolda dolan GPU sonrakileri buluta
    iter. Config validate: tek-token, benzersiz, boş olmayan liste + core task
    (plan/code/verify/report) listeye giremez testi.

## Faz 4-a — Kalıcılık temelleri — DONE (bu oturum)
- `src/clock.rs`: bağımlılıksız UTC `now_utc_iso8601` (Hinnant civil_from_days). `fmt_epoch_seconds` saf → testler wall-clock'la yarışmaz (Python timegm ile çapraz doğrulanmış sabitler).
- `src/events.rs`: **EventLog** — `runs/<spec_id>/events.jsonl` append-only; `seq` run-içi monotonik (§7.3); her append `write_all + sync_all` (kill→resume olay kaybı yok); `open` mevcut satırları tarayıp `last_seq`'i devam ettirir, yarım (corrupt) satırı warn'la atlar, yabancı `run_id`'yi saymaz. `replay()` run'a göre filtreler.
- `src/lock.rs`: **RunLock** — `.ribozym/lock` (pid + heartbeat mtime); `create_new` atomik → iki kopya yarışamaz; taze mtime'i canlı tutan reddeder (HeldBy), çökmüş (stale) kilit geri alınır ve `stolen=true` döner (çağrı `LockStolen` olayı yazar). pid-liveness OS sorgusu yerine heartbeat-stale (bağımlılıksız, Win/Linux aynı). `Drop` yalnız kendi pid'i eşleşirse siler.
- Test: clock 6 + events 5 + lock 6 = 17 yeni; toplam **45 unit + 13 integration = 58**, clippy 0, skeleton 52/52.

## Faz 4-b/4-c — Orkestratörün kalanı — DONE (bu oturum)
- `src/hash.rs`: bağımlılıksız **SHA-256** + `canonical_json` (key-sort'lu, bayt-stable). `payload_hash` (§7.4) ve golden-hash temeli. Vektörler Python `hashlib` ile çapraz-doğrulandı (63/64/65/119/200 bayt — blok sınırları testli).
- `src/ledger.rs`: **op ledger** — `op_id` payload'dan deterministik (`kind|spec|task|hash`), böylece "zaten finished → atla" (rule 4) tarama değil PK kontrolü. `start` = `INSERT OR IGNORE` (çökme-penceresinde yeniden-türetim ikinci satır uydurmaz); `set_result`/`finish`/`fail`; `plan_resume` §7.4'ün dört dalını saf fonksiyon olarak verir (agent_call+result→FinalizeResult, shell/git/web→ReRun, closed→Done).
- `src/scheduler.rs`: **DAG zamanlayıcı** (§6.2) — saf veri yapısı (spawn/I-O yok). Kahn ile döngü/unknown-dep/duplicate reddi; `admit()` = kapak + tek-yazar (kesişen artifact yolları sıraya girer, `TaskOwnershipConflict` eşleri döner); `mark_unreachable` başarısızlığı `Blocked`'a kaskadlar (çalışan iş iptal edilmez, pending soy türeyenler bloklanır); `all_terminal` "≥1 done" ister; kritik-yol doğrulanmazsa yeniden hesaplanır. Fuzz: 500 deterministik rastgele DAG — ya döngü reddi ya tam drain.
- `src/gates.rs`: **§9 kapıları** — otonomi matrisi + batch + Q4 timeout politikası saf `decide()`; `pending` satırı = kalıcı İSTEK kaydı (timeout saati süreç sınırını aşar). `approvals` tablosu + HumanApproval* olayları.
- `src/orchestrator.rs`: **sürücü** — FSM (§6.1) + DAG + ledger + kapılar + event log + state.db. §7.4 kesin yazım sırası TEK yolda (`commit_task`); `arm_crash(Stage)` ile her B4 penceresi enjekte edilebilir. `resume` = unfinished ledger'ı kapat (FinalizeResult executor'a DOKUNMAZ; ReRun in_progress→pending) + FSM/scheduler'ı DB'den rebuild. `final_digest` = normalize edilmiş son-durum hash'i (tasks+ops+event-SET; seq/ts/run_id hariç → idempotent resume aynı hash'i üretir).
- CLI: `run --mode mini --tasks <json> [--yes] [--autonomy N]` + `approve <spec> <gate> [--deny --note]` (§9 dosya-üstü oy); §20.2 çıkış kodları; `RunLock` tek-kopya disiplini; `MockExecutor` (Faz 5'te A-ajan hattı yerini alacak). Binary smoke: `delivered` + hash üretti, exit 0.
- Stats: **98 unit + 8 golden + 13 integration = 119 test**, clippy `--all-targets` 0, fmt clean, skeleton 52/52.

## Faz 5-a — Ajan MVP'si ilk dilim: A5 LeadCoder + AgentBridge — DONE (bu oturum)
- **Köprü kararı (önceki "karar bekliyor" çözüldü):** kontrol düzlemi senkron kalır (golden-digest determinizmi, tek-yazar), ajanlar `async_trait` (§5.3). `agents/bridge.rs` → `AgentBridge`: her görev için **ayrı current-thread tokio runtime + `block_on`** — paylaşılan reactor yok, ajan zamanlaması determinizmi bozamaz.
- **`schemas/coding_task.schema.json` (yeni, §11.8):** A5'in LLM çıktı sözleşmesi — `files[{path,content}]` + `self_check.commands` + `task_id`. Yol kuralı şemada bile (traversal/absolute reddi); pozitif fixture + negatif `coding_task_path_traversal.json` eklendi → **skeleton 52→55 kontrol**.
- **`agents/a5.rs` — A5 LeadCoder (§5.10):** prompt dosyasından (§11.7 `prompt_file`) byte-stable system prefix; şema-zorunlu LLM turu (SchemaGuard, §14); patch'ler **izole workdir**'e yazılır (`safe_path`: `..`/absolute/`tests/` reddi — §5.9 tests/ A4'ündür, şema geçse de kod katmanında ikinci savunma); `self_check` komutları **yalnız ToolGateway'ten** (allowlist, izole env); **fix döngüsü ≤3 toplam tur (§6.3/§8.1)**, tükenirse `status:"failed"` ExecutionResult döner (panic değil — kontrol düzlemi TaskFailed + blocked kaskadı yapar, ladder sahiplenir).
- **Dürüst muhasebe:** `model_usage.cost_usd` gerçek provider usage'ından; `spent_usd` ondan beslenir (task'ın declared estimate'inden DEĞİL — test bunu ayrıca kanıtlar). Mock motor $0 faturalandırır.
- **E2e (`tests/agents_e2e.rs`, LLM mock — her şey başka gerçek, §21.1):**
  1. iki zincirli görev → model dosyası workdir'e **gerçekten** yazılır + **gerçek `cargo check`** geçer → Delivered; ledger'da 2 kapalı agent_call.
  2. prose-sarmalı bozuk ilk yanıt → §14 repair turu çalışır (3 çağrı / 2 görev) → yine Delivered.
  3. derlenmeyen kod → fix-loop 3 turda tükenir → `runs.status='failed'` persist, bağımlı görev `blocked`, çağrı sayısı tam 3 (tavan aşılmaz).
  4. gerçek `cargo check` allowlist'ten geçer; `cargo publish` reddedilir.
- Stats: 115 unit + 4 agent-e2e + 6 budget + 8 golden + 13 integration = **146 test**, clippy `-D warnings` 0, fmt temiz, skeleton **55**/55.

## Faz 5-b — A7 Security + A4 Test Architect + §6.3 hattı bütünü — DONE (bu oturum)
- **A7 Isolation & Security** (`agents/a7.rs`, §5.12): ağırlıkla **deterministik** (m3 antivirüs modu):
  - **Secret taraması workdir'in TAMAMINDA** (target/.git/.ribozym hariç; text uzantılar) — task'ın beyan ettiği dosya setiyle sınırlı DEĞİL; desener dar tutulur (her desene regresyon testi şartı). Kritik bulgu → `SECURITY_SECRET_FOUND` TaskFailed olayı akışına düşer.
  - **İzolasyon denetimi**: plan `self_check_commands` taşıyorsa gateway izin listesine bağımsız yeniden sorgulanır (unit testli; `rm -rf /` → breach).
  - **cargo audit**: ÖNCE `cargo audit --version` capability-probe (araç yok = exit 101'li audit-failure'ından ayrışır; §12.4 warn+skip), sonra gerçek audit — allowlist'e probe argv'sı eklendi.
  - `model_usage: null` (§11.3 deterministik ajan kuralı), LLM kullanmaz.
- **A4 Test Architect** (`agents/a4.rs`, §5.9): `tests/` mülkiyeti TERS çevrilmiş safe_path (yalnız tests/), **hakkaniyet kuralı m5(a) kod-düzeyinde**: üretilen test dosyası AC-id'yi (task.extra.acceptance_ids) doc-comment'te **cite etmezse yazılmadan reddedilir**; test koşumu `cargo test` gerçek gateway üzerinden (yazım LLM, koşum yok — m4). Fairness e2e'de kanıtlanır: aynı test seed crate'te GERÇEKTEN düşer, A5'in koduyla geçer.
- **§6.3 hattı bütünü (e2e, `tests/pipeline_e2e.rs`)**: A5(kod)→A7(security)→A4(test) DAG sırasıyla dispatch; A7 LLM'siz; test gerçekten koşar; 3 ajanın op'ları ledger'da kapalı.
- **Üç gerçek hata bulundu & düzeltildi (bu dilimin review'ı):**
  | # | Bulgu | Düzeltme | Kanıt |
  |---|---|---|---|
  | P1 | `cargo audit` araç yokken exit 101 → "audit başarısız" sanılıp görev çökertilirdi (araç-yokluğu ile bulgu ayrışımsız) | `cargo audit --version` capability-probe (allowlist'e eklendi) → araç yoksa §12.4 warn+skip | e2e: audit'süz makinede hat Delivered |
  | P2 | **Failed/blocked görevi olan run, blanket `compliance_ok=true` ile Delivered olabiliyordu** — kırmızı işin üstünde yeşil ışık (dürüstlük ihlali) | FinalReview: görev-tümü-done değilse compliance ZORLA reddedilir (scheduler gerçeği > çağıranın beyanı); crisis-cap → sonlu NeedsUser | `secret_in_code_is_critical...`, `untraceable_test_is_rejected` |
  | P3 | Ajanın semantik hatası (izlenebilir olmayan test, yanlış task_id) bridge'den `?` ile fırlayıp **sürücüyü çökertiyordu** | Bridge, agent Err'ini failed Executed'e çevirir → TaskFailed + blocked kaskadı (§8.1 ladder girişi) | `untraceable_test_is_rejected` (test dosyası YAZILMAMIŞ + run sonlu) |
- `execution_result.schema.json` provider enum'una `mock` eklendi (mock motor gerçek; $0 faturalanır — dev-döngüde model_usage null değil dürüst 0).
- Stats: 117 unit + 4 agents-e2e + 3 pipeline-e2e + 6 budget + 8 golden + 13 integration = **151 test**, clippy `-D warnings` 0, fmt temiz, skeleton 55/55.

## Faz 5-c — Yönetim hattı: A1 + A2 + A3 + spec_match — DONE (bu oturum)
- **A1 Baş Mimar & Hakem** (`agents/a1.rs`, §5.4C): iki şapka, kontrol-düzlemi
  çağrısı (DAG görevi DEĞİL):
  - `draft_spec`: form BÜTÜN HALİNDE tek çağrı (§5.4 aşama 2). **Provenance
    koddan**: `source.raw_input_hash` gerçek baytların sha256'sıyla EZİLİR —
    model ne iddia ederse etsin (§5.4C sessiz tahmin yasağının kanıt tarafı).
  - `final_review`: ComplianceCheck. `adjudicate` saf fonksiyonu: pct **kodun
    paydasıyla** yeniden hesaplanır (met=1, partially=0.5), uydurma req-id'leri
    atılır, gözardı edilen req = unmet, denetlenmemiş varsayım = İHLAL SAYILIR
    (konservatif tamlama, §5.4C maddi cezası −10), `unmet_requirements`
    status'lardan YENİDEN kurulur, verdict tavanı: sub-100'de `approved`
    olamaz (P5); model kendinden sertse sertliği kalır.
- **A2 CEO** (`agents/a2.rs`, §5.6A): `make_plan` — LLM içeriği/sırayı önerir,
  sözleşmeyi kod yazar: §6.4 izlenebilirlik + DAG (Scheduler) + **atanabilirlik**
  (kayıtta olmayan ajan PLANDA reddedilir) + **tahmin uydurmaz** (LLM'in
  yazdığı `estimated`'lar null'a zorlanır, task_scores tek kaynak — Faz 8) +
  **taze plan pending doğar** + türetilmiş görünümler (execution_order,
  critical_path, status_summary) koddan.
- **A3 Baş Raportör** (`agents/a3.rs`, §5.7): `skeleton` saf kod (görev
  gerçekleri state.db + events'ten; provider bazlı maliyet kovaları;
  §6.4 izlenebilirlik haritası; blocked/skipped dürüst sayımı + warning).
  `narrate`: LLM'e iskelet gider, **yalnız anlatı alanları** geri alınır —
  sayıları model yeniden yazamaz (test: 999$ yalanı → 0.01$ gerçeği kalır;
  motor hatası → sayısal rapor yine çıkar). `seal`: A1 hükmü gömülür,
  teslimat kapısından ÖNCE (§5.7 m4); `agrees_with_work` ikinci kilit.
- **Model**: `src/models/compliance.rs` (ComplianceCheck + `delivery_ok()`
  tek yetkili yargı — pct tek başına karar veremez).
- **Dikiş**: `Orchestrator::advance_with(exec, &mut dyn FinalPipeline)` —
  `Finalizing`→A3, `FinalReview`→A1 (kriz yeniden-hükmü dahil). `advance`
  aynen kaldı (`FixedCompliance` sarmalayıcı) → **golden hash'ler byte-bayt
  korundu**. `bridge` özeti artık model/provider/tokens/cost/cache_savings
  taşıyor (A3 sayıları olay akışının kendisinden okur).
- **`src/pipeline.rs`**: `plan_from_form` (form→A1→A2, run satırından ÖNCE;
  reddedilen plan state.db'ye dokunmaz) + `AgentPipeline` (facts koleksiyonu,
  kanıtları KOD kurar, A1'e gider; mühürlü rapor + `compliance.json`
  ayrı denetim dokümanı; resume'da raporu diskten geri yükler). Her hat
  çağrısı köprüyle aynı dikiş: ayrı current-thread runtime, kontrol düzlemi
  senkron kalır (§5.3/§7.5).
- E2E (`tests/planning_e2e.rs`): form→delivered tam hat (mühürlü rapor
  şema-geçerli, sayılar koddan, anlatı modelden, izlenebilirlik haritası);
  iki taze run → AYNI final hash; **yalancı hakem** (100 iddiası + unmet
  status) → NeedsUser sonlu, raporda mühürlü %0 + tasks_done=3 BERABER;
  `examples/spec_rest_api.md` golden planı (3 req tam kapsanır, estymate'ler
  null, topological sıra koddan).
- **Review bu oturum (3 gerçek açıklık bulundu & testle kilitlendi):**
  | # | Bulgu | Düzeltme | Kanıt |
  |---|---|---|---|
  | A-c1 | pct modelin YAZDIĞI satırlar üzerinden hesaplanıyordu — uydurma `R99: met` gerçek %0'ı %66'ya şişirebilirdi | payday = spec'in gereksinim kümesi; spek-te-olmayan verdict'lar atılır (warn) | `fabricated_requirement_cannot_inflate_the_score` |
  | A-c2 | Boş `assumptions_audit` §5.4C cezasından muaf tutuyordu (sessizlik = yeşil); review girdisi yalnız requirements taşıyordu, varsayımlar denetlenemezdi | denetlenmemiş varsayım ihlal sayılır; girdide assumptions+decisions tam | `unaudited_assumption_counts_as_falsified` |
  | A-c3 | Model plan.json'a `status:"done"` yazabiliyordu (DB satırları pending — diskteki yalan belge kalırdı) | taze plan doğuşta pending | `a_fresh_plan_is_born_pending_whatever_the_model_claims` |
- Stats: **135 unit + 4 agents-e2e + 3 pipeline-e2e + 4 planning-e2e + 6 budget
  + 8 golden + 13 integration = 173 test**, clippy `-D warnings` 0, fmt temiz,
  skeleton 55/55; CLI mini smoke değişmedi (exit 0, `advance` sarmalayıcısı).
- Borç notları: spec-kapısı yaşam-döngüsü (Clarifying→…→SpecValidated olayları
  ve `--assume-compliant`'ın CLI'dan kalkması) hâlâ mini-yolda; `pipeline`
  şema-doğrulamada repo-root fallback'i cwd'ye bağlı; başarısız görevin
  maliyeti run-toplamına giriyor ama per-task `cost_usd` kayıt-hatı değil;
  `decisions_compliance` A7/A4 kod-kanıtıyla henüz ölçülmüyor (model beyanı).

## Faz 5-d — Yaşam-döngüsü kapanışı + muhasebe bütünlüğü — DONE (bu oturum)
- **§6.1 baştan sona sürüklüyor**: `Orchestrator::create_idle` +
  `advance_planning(&mut dyn PlanningPipeline)` — Idle→SpecReceived(form
  kaydı + KOD denetimi)→Clarifying(non-interaktif hızlı yol)→Researching
  (A1 draft)→Advising(lock denetimi: decisions+reqs+bütçe boşsa sessiz
  devam YOK, sonlu NeedsUser)→**spec gate (gerçek §9: pending satırı, timeout
  ankeri, veto/park yolları canlı)**→TasksCreated→Scheduling. Olay akışı
  artık run'ın TÜM hikâyesini taşıyor (SpecReceived < SpecValidated <
  TasksCreated < Delivered seq-testi).
- **`validate::check_form`**: §5.4 form bütünlüğünün KOD denetimi (başlık,
  7 bölüm, en az bir non-empty `must_have`, pozitif `run_budget_usd`,
  HTML-yorumları cevap sayılmaz). Eksik form model görmeden park olur —
  e2e: BOŞ mock script'li test, modele tek tur gitmediğini kanıtlıyor.
- **`run_op` / `plan_turn` (§7.4 muhasebesi yönetim turlarına da)**: A1
  draft, A2 plan, A3 narrate, A1 review — dördü de deterministic op
  kimliği + persisted envelope + exactly-once charge; `plan_resume` dalı
  `agent_call|plan_turn + result` → FinalizeResult. Kriz yeniden-hükmü
  aynı kanıt-setiyle aynı op_id'yi üretir → **model tekrar faturalanmaz**
  (e2e: a1-review ledger satırı = 1, NeedsUser üç review turuna rağmen).
- **Spec bütçesi artık kural**: `set_run_budget` — draft'lanan spec'in
  `budget.max_cost_usd`'si run satırının duvarı olur (§8.2 config varsayılanı
  yalnız spec-öncesi); CLI resume satırdaki bütçeyi koruyor.
- **A1/A2/A3 turları artık `TurnUsage` taşıyor** (`inference::TurnUsage`,
  fiyat tablosu `inference::estimate_cost`'a TAŞINDI — katman hiyerarşisi:
  inference agents'a bakmaz; a5 delegate korundu). Yönetim işi bedava değil.
- **CLI**: `run --mode interactive --form <md> [--spec-id] [--workdir] [--yes]`
  — gerçek sağlayıcı yığını: cloud birincil (Q1), local grunt-offload
  `LocalFirstRouting` ile; A10 gelene geçici seçim = registry'den ilk
  kara-listesiz model (kaynak-notlu). Anahtarsız: net CONFIG_INVALID → exit 1
  (state'e dokunmadan). RunLock + resume + §20.2 kodları mini ile aynı.
- **Review bu oturum (gerçek hatalar, testle kilitli):**
  | # | Bulgu (gerçek hata) | Düzeltme | Kanıt |
  |---|---|---|---|
  | Q1 | **Çift-şarj**: AfterDoneTx/AfterDoneEvent penceresinde kill → resume'da FinalizeResult dalı `runs.spent_usd`'e kalıcı maliyeti TEKRAR yazardı (mirror'dan mutlak yazım; satır şarjı zaten taşıyordu). Mini golden'lar cost=0 ile koştuğu için HİÇ görünmedi — "kill ile çift-şarj yok" iddiası kanıtsızdı | migration 0003 `ledger.charged_at` + `charge_once` + GÖRELİ `spent_usd = spent_usd + ?` AYNI tx'te (rollback guard tüketemez) | `kill_in_commit_window_charges_exactly_once_across_resume` |
  | Q2 | `budget_warned` latch'i HER commit'te `crosses_soft`'un 0/1'ini yazıyordu — sonraki bir commit uyarı rayasını SİLİYOR, süreç-sonrası yeniden-uyarıya yol açıyordu | `MAX(budget_warned, ?)` sticky yazım | `soft_latch_is_sticky_across_process_boundaries` (3 proses, tek BudgetWarning) |
  | Q3 | **Determinizm kaçağı**: A3 süreleri olay zarfının duvar-saati `ts`'inden türetiliyordu → skeleton → narrate payload'ı → ledger DISTINCT-tuple → **aynı iki run farklı final hash**. Eski golden bunu göremedi (mini hattı ts okumuyor) | süreler LEDGER started/finished damgalarından (sürücünün enjekte-saati domaini) okunur | `two_identical_runs_produce_identical_final_digests` (3x koşum, stabil) |
  | Q4 | `absorb_plan` yoktu: spec gate sonrası bellek-içi scheduler boş kalıp run'u "no task can reach done" ile çökertirdi | kapı geçince by_id+sched DB'den yeniden kurulur | planning e2e delivered + `planning_lifecycle_walks_every_phase` |
  | Q5 | `check_form` ilk halinde byte-aritmetik section dilimleme vardı (yanlış slice/panic riski) | satır-tabanlı `sections()` taraması; Unicode literal | form testleri 4/4 |
  | Q6 | Test enstrümantasyonu hatası: `calls` sayacı work closure DIŞINDAYDI — rule-4 atlama "yeniden çağrı" gibi sayılıyordu (yanlış pozitif) | sayaç closure İÇİNE taşındı | `plan_turn_crash_resume_never_re_invokes_the_model` |
- Stats: **145 unit + 4 agents-e2e + 8 budget + 8 golden + 13 integration +
  3 pipeline-e2e + 5 planning-e2e = 186 test**, clippy `-D warnings` 0, fmt
  temiz, skeleton 55/55; CLI smoke: interactive anahtarsız → net hata exit 1
  (state'e dokunmaz), mini unchanged exit 0.

## Faz 6-a — Gece sertleştirmesi: para güvenliği + yaşam-döngüsü bütünlüğü + A11 — DONE (bu oturum)
- **P1 — EnvLayer sıcak yolda (gerçek anahtar para güvenliği):** CLI provider
  yığınının üstüne `EnvLayer(price_of(cloud), budget − spent_satırı)` bindirildi —
  çağrı-öncesi pre-clamp artık yaşıyor; `ProviderError::Budget` AYRI varyant
  (para reddi transient değil → E0 backoff YOK); bridge reddi algılar → görev
  FAIL sayılmaz, run `Paused(budget)` + reddedilen görev `pending`'e döner;
  planlama turları duvara çarparsa `PlanningOutcome::Paused`. Kanıt:
  `wall_hit_at_call_boundary_pauses_then_resumes`,
  `planning_wall_hit_pauses_at_the_same_phase_and_resumes`, 2 env uniti.
- **P2 — Okuma yüzeyleri (yazılmış gerçeği projekte et, asla yeniden
  hesaplama):** `ribozym status [spec]` (faz/görev sayaçları/harcama/açık kapı +
  arşiv taraması), `report --format md|json` (mühürlü belgenin deterministik
  markdown'ı — yeni içerik ÜRETMEZ), `budget show`, `new-form` (var olan formu
  ezmez). `-0.0000` tuzağı: std `f64::sum` boş yığında `-0.0` döndürüyor (NaN
  koruma tohumu) → `fold`.
- **P3 — A12 arşiv (§7.6/B2):** `archive/NNN-ad` bölüm + `manifest.json`
  (sha256+boyut+ts); yazımda read-back doğrulama; `append` = değişen bayt
  sonraki numaraya yazılır, TARİH ASLA yeniden yazılmaz; `verify_all`
  status/resume taraması; `repair_section` kaynağı varsa yeniden mühürler,
  yoksa `false` → NeedsUser yolu. Yaşam belgeleri (form/spec/plan/compliance/
  mühürlü rapor) bölüm bölüm mühürlenir; okumada drift `ARCHIVE_CORRUPT`
  (sessiz-devam yok). Sapma notu (dürüst): zstd bağımlılığı yok — ham bayt,
  manifest protokolü aynı, sıkıştırma sonradan takılır.
- **P4 — E2 kriz replanı GERÇEK:** `FinalPipeline::replan` kancası (varsayılan
  NoChange → mini altınları aynen), FSM'e `Trigger::Replanned`
  (Crisis→Scheduling, `TasksRevised` olayı) + exhaustive-tablo testine işlendi;
  `A2Ceo::replan`: done görevler (id+ajan+req atıfları) bayt-baz koşuluyla
  korunmak zorunda, gereksinim seti değişemez (kapsam → suggest), revision =
  kodun `old+1` damgası, tur kimliği (plan+verdict+done-set hash'i) → kural-4:
  çökmüş kriz turu modeli yeniden faturalamaz. `absorb_revised_plan`: done
  satırlarına dokunmaz, failed/blocked'u pending+retry'ye çeker, düşürülen
  görev satırı silinir. E2E:
  `crisis_replan_recovers_and_delivers_on_the_second_verdict` (gap → T4
  kurtarma → %100 → delivered; revizyonlar arşivde 003+005 olarak duruyor).
- **P5 — A11 web kapısı (§13, Q2 kararı gereği rules_only):** `WebFetcher`
  trait + `NullFetcher` (sessiz sahte-arama YOK: `WEB_UNAVAILABLE`) +
  `CommandFetcher` (argv şablonu, `{ask}` TEK argv elemanı — shell yok;
  timeout+cap; SSRF muhafızı loopback/metadata reddi); normalize-ask cache
  anahtarı, TTL içinde yeniden-puanlama YOK (`verified_by` mühürlü skor),
  isabet zarfının `query_id`'si istekle yeniden mühürlenir; kural katmanı
  (domain≥2→40, tip ağırlığı×30, yaş≤24ay×30 → band), injection taraması →
  ağırlık SIFIR + confidence zorla low; snippet yerine SADECE sha256 (P7:
  içerik veridir). `A11Web` registry'de gerçek DAG görevi: şema-kapılı WebQuery
  girdisi, şema-doğrulanmış çıktı, `model_usage:null` (§11.3), bridge
  `.with_web()` builder'ı (mevcut kurulum noktaları kırılmadı). web_e2e 5 test.
- **P6 — Yayın cilası:** TR README (5-dakika akış, çıkış kodları, anatomi,
  güvenlik taahhütleri, açık kalanlar), `CHANGELOG.md` (0.1.0: eklenenler +
  gece bulunan GERÇEK hatalar + sapmalar), `scripts/gates.sh` (4 kapı = CI),
  release build + `--version`/`config validate` binary smoke.
- **Review bulguları (bu oturum, testle kilitli):**
  | # | Bulgu | Düzeltme | Kanıt |
  |---|---|---|---|
  | N1 | Para reddi `Unavailable`'a mapiyordu — backoff reddedilen çağrıya uyurdu | `ProviderError::Budget` ayrık, non-retryable | `budget_refusal_is_a_dedicated_non_retryable_error` |
  | N2 | Cache isabeti eski `query_id` zarfıyla dönüyordu (kayıt sorguyu yanlış söylüyordu) | içerik/skor aynen, id yeniden-mühür | web_e2e cache testi |
  | N3 | std `sum()` boş ledgerde `-0.0`; `budget show` "-0.0000" basıyordu | `fold(0.0, +cost)` | surface test + CLI smoke |
  | N4 | A11'in bulgu dosyaları `safe_path`'in kopyasını istiyordu (iki doğruluk kaynağı riski) | A5'in tek kapısı `safe_path_pub` ile yeniden kullanıldı | a11 kayıt yazımı |
- Stats: **167 unit + 4 agents-e2e + 9 budget + 8 golden + 13 integration +
  3 pipeline-e2e + 8 planning-e2e + 5 web-e2e = 217 test**, clippy
  `-D warnings` 0, fmt temiz, iskelet 55/55, `scripts/gates.sh` tek komut =
  CI, release binary çalışıyor.

## Sıradaki — Faz 6-b/7/8
Kalanlar: `ribozym interview` Q&A + `suggest` kanalı (§5.5 kart döngüsü), A6
araştırma zinciri (A11 `WebQuery` ile), A8 yazar/repo + A9 sürüm (git güvenliği
§12.5 ile), A10 calibrate/tier-list + `task_scores` + `prices`, arşivde zstd
(bağımlılık penceresi açılınca), brownfield kopya izolasyonu (Q3) ve
`logs/attach/dry-run` CLI yüzeyleri. Yayına hazırlık notu: mevcut hâl
yeşil-kapılı tek-akış (form→delivered) + deterministik mini hat + okuma
yüzeyleriyle KULLANILABİLİR durumda; kalanlar genişleme — çekirdek güvenlik ağı
(para/izin/şema/ledger/arşiv) tam ve testli.
## Code review pass — önceki oturum (Faz 2–4-a): AI-slop taraması, gerçek hatalar bulundu & düzeltildi
Acımasız self-review; her bulgu bir testle kilitlendi (regresyon koruması):

| # | Bulgu (gerçek hata/dublasyon) | Düzeltme | Kanıt |
|---|---|---|---|
| F4 | `extract_json` ilk `{` başarısızsa `None` dönüyordu — repair turu prose'deki sahte küme (`use {format}`) sonrası gerçek objeye asla ulaşamazdı (döngü mantıksal olarak bozuktu) | Her `{` aday denenir; `balanced_object` ayrıldı | `extract_json_skips_decoy_braces_before_the_real_object` + 3 ek test |
| F1 | `cloud.rs`'te `ApiResponse` ≡ `ApiResponseShape` kopya; aynı protokol iki kez parse ediliyordu (drift riski) | Silindi; ortak `to_response(provider, fallback_model)` | cloud + local aynı yoldan assemble |
| F7 | `SystemRunner` `.stdout(piped())` YOKTU → `child.stdout.take()` = `None`, tüm araç çıktısı sessizce kayboluyordu; ayrıca iki pipe sırayla okunuyordu (stderr buffer dolunca klasik pipe-deadlock); timeout'ta süreç reap edilmiyordu (zombi) | piped() eklendi; her pipe ayrı thread; kill→wait→join; argv-empty koruması | `system_runner_captures_real_output` (gerçek `cargo --version` stdout) |
| F5 | `EnvLayer` bütçe düşümü load-then-sub (concurrency'da ölü bütçeyi diriltme riski) | `compare_exchange_weak` + `saturating_sub` loop | tek-yazar kontrol düzlemi + kilit ile birlikte savunma |
| F6 | `Err(e @ RateLimited)` bağlamış ama `let _ = e;` ile atılıyordu (ölü binding + log yok) | Kaldırıldı; E0 retry'ları `tracing::warn` ile görünür | derleme uyarısı + §15 log |
| F8 | Inherent `complete` ve trait `complete` aynı ad (sessiz overload) | Inherent → `complete_guarded` | isim çakışması bitti |
| F-new | Bütçe reddi `ConfigInvalid` olarak işaretliydi (§16'da ayrı sınıf var) | `Error::ModelBudgetDenied` + trait yüzünde Unavailable-noted | §16 taksonomisi |
| F11 | Local→cloud fallback'i tamamen sessizdi (telemetri yok) | `tracing::warn!` (run'a sessiz, log'a görünür) | §15 |
| F13 | `models.toml [artificial_analysis]` alanı struct'ta yoktu (sessizce yok sayılırdı) | `ArtificialAnalysisSection` eklendi + config round-trip | parse |
| — | `select.rs` unused import | temizlendi | clippy 0 |
Test: 25 unit + 13 integration = **42**, clippy `--all-targets` 0, fmt temiz, skeleton 52/52.

## Code review pass — bu oturum (Faz 4-b/4-c): gerçek hatalar bulundu & düzeltildi
Yeni orkestratör/ledger/scheduler/gate kodunda acımasız self-review; her bulgu testle kilitlendi:

| # | Bulgu (gerçek hata) | Düzeltme | Kanıt (test) |
|---|---|---|---|
| R1 | `mark_unreachable`'ın `blocked` kaskadı yalnız bellek-içi zamanlayıcıda kalıyordu → `tasks` tablosu stale 'pending' gösterir; resume çelişkili rebuild ederdi (§7.1 tek-gerçeklik ihlali) | Cascade, başarısız satırla AYNI state.db transaction'ında yazılır (clone üstünde hesaplanır, ayna sona kalır) | `all_failed_cascades_blocks_and_persists_terminal_phase` |
| R2 | "hiçbir görev done olamaz" durumunda `advance` `Failed` dönerdi ama `runs.status` 'scheduling'de asılı kalırdı — resume ölü döngüye yeniden girerdi (§6.1 Failed terminal) | Stalled dalında `Trigger::FatalError` ateşlenir → 'failed' persist | aynı test (`status = 'failed'` assert) |
| R3 | Ledger `finished` op'su, task satırı sıfırlanmışsa (yanlışlıkla ikinci `create`) yeniden KOŞABİLİRDİ — §7.4 rule 4'ün kanıtı olan ledger kontrolü eksikti | `run_one_task` başında `is_finished` guard'ı; geçtiyse DB+sched `done`'a geri yazılır | `finished_ops_never_re_run_even_after_misused_recreate` (sıfır executor çağrısı + aynı hash) |
| R4 | Başarısız görev için de `result_ref` yazılıyordu — AfterWork penceresinde çökme, `FinalizeResult` dalıyla hatayı SESSİZCE başarıya çevirirdi | `persist_result` yalnız `ok=true` için; `hit(AfterWork)` de o dala taşındı | aynı test: artifacts dizininin OLUŞMADIĞI assert'i |
| R5 | `HumanApprovalRequested` olayındaki `repeated` alanı, poll sayısını golden hash'e sızdırıyordu — farklı çökme zamanlarında hash kayardı | Alan kaldırıldı (tekrar sayısı zaten dosyada okunur) | `golden_kill_resume_converges_from_every_write_window` (5 pencere aynı hash) |
| R6 | `decide` mevcut `approved` satırının sebebini BAĞLAMDAN yeniden türetiyordu: `--yes`'le geçmiş kapı sonraki süreçte "human" görünürdü — §9 "kim, neyle" denetim izi yalan söylerdi (golden hash testinin düşmesiyle yakalandı) | `approved_reason(decided_by)`: provenance satırdan geri okunur | `grant_reason_is_read_from_audit_not_re_derived_from_mode` + R3 testinin hash-eşitliği |
| R7 | `topological_order`'da seçilen düğüm `indeg`'te 0 kalıyordu → aynı düğüm tekrar seçilip bağımlının sayacı **negatife** iniyordu (panic) | `done` kümesi eklendi + `saturating_sub` | scheduler tam suite (regresyonda patlayan testler düzeldi) |
| — | FSM'in §9 kapı-veto/park yolları tabloda yoktu (delivery reddi NeedsUser'a taşınamıyordu) | `Trigger::GateDenied` + `GateParked` + geçişler | `gate_denial_and_park_reaches_needs_user` + exhaustive-determinism güncellendi |
| — | clock'ta ISO→epoch tersi yoktu (gate timeout ankoru okunamaz) | `epoch_from_utc_iso` (alan aralığı doğrulamalı; çöp → None) | `parse_is_the_exact_inverse_of_format` / `parse_rejects_garbage_rather_than_guessing` |

Test: 98 unit + 8 golden + 13 integration = **119**, clippy `--all-targets` 0, fmt temiz, skeleton 52/52.

## Faz 4 borç-kapatma + "dekoratif/sabit-kod" denetimi — DONE (bu oturum)
Analizde çıkan borçlar kapatıldı; sisteme "işlevsiz görünüp çalışmayan" hiçbir şey girmediği denetlendi.

- **§6.4 izlenebilirlik kapısı** (`validate::check_traceability`): her `requirements[].id` ≥1 task tarafından anılmalı + task'lar olmayan req-id'si cite edemez + spec↔plan id uyuşmalı. `Orchestrator::create_with_spec` bunu `TasksCreated`'ten ÖNCE uygular; reddederse HİÇBİR run/tasks satırı yazılmaz (yarım-kayıt yok). CLI `--spec` verildiğinde canlı; verilmezse "SKIPPED" high-confidence bildirilir (sessiz geçmez). Negatif: `TASK_UNCOVERED_REQUIREMENT`/`UNKNOWN_REQUIREMENT_REFERENCE`/`SPEC_ID_MISMATCH`. Not: `examples/expected/negative/tasks_uncovered_requirement.json` aslında **schema-level** "öksüz task" vakasıdır (minItems:1) — cross-doc kuralı ayrı test edildi; isimlendirme yanılgısı docstring'e not düşüldü.
- **§8.2 run düzeyi bütçe kelepçesi** — `spent_usd`/`budget_warned`/`paused_from`/`paused_reason` artık **canlı** (önceden dekoratif kolonlardı):
  - `Executed.cost_usd` → `commit_task` transaction'ında `runs.spent_usd`'e birikir (kill ile çift-şarj/kayıp yok; §7.1).
  - **Pre-clamp**: bir sonraki task'ın **declared estimate**'i sert duvarı aşıyorsa task hiç başlatılmaz → `Trigger::BudgetHardLimit` → `Outcome::Paused(budget)`, `runs.status='paused'`, `paused_from` persist. Estimate yoksa (None) uydurulmaz (§5.6 "tahmin uydurmaz") → o task call-boundary'de EnvLayer tarafından durdurulur.
  - **Soft %80**: geçişte TEK `BudgetWarning` olayı; `budget_warned` latch'i DB'de → pause→resume sınırını aşar, spam yok (testli).
  - **Resume**: `Orchestrator::unpause()` bütçe yükseltilince `Trigger::Resume` ile `paused_from`'a döner; T1/T2 tekrar KOŞMAZ (rule 4). CLI resume'da otomatik dener.
- **FSM**: `RunState.paused_from` + `Trigger::Resume`; `GateDenied`/`GateParked` yolları. Pause ankersız Resume → NoTransition (Idle'a ışınlanma yok).
- **migration 0002** (`0002_budget_pause.sql`): `paused_from/paused_reason/budget_warned` kolonları; `state.rs` test-paritesi 0001+0002'yi birlikte gömer (on-disk runner ile byte-içerik).
- **§15 logging**: bağımlılıksız `logging.rs` — elle `tracing::Subscriber` (WARN+ → `ts|LEVEL|target|msg k=v`). `tracing-subscriber`/`-serde` offline registry'de YOK; bu yüzden minimal kendi subscriber'ımız. CLI'da stderr'e kurulur; **events.jsonl'e asla yazmaz** (run kaydı ile işlem log'u ayrı kalır). Önceden "ölü" olan `tracing::warn` çağrıları (rule-4 skip, local→cloud fallback, E0 retry, stale-lock) artık gerçekten bir yere gidiyor.
- **CLI dürüstlük**: `compliance_ok` sabit `true` KALKTI → `--assume-compliant` bayrağı (kapalıyken FinalReview → Crisis tavanı → sonlu `needs_user`, asla sonsuz döngü; e2e doğrulandı). `Waiting`→exit **4** (0/decorative-değil), `Failed`→exit **2** (kullanıcı-hatası 1 DEĞİL — gerekçeli), `Paused`→exit **3** (BUDGET). `approve` öksüz spec'i reddeder (run satırı yoksa exit 1). Kullanıcı-dosyası parse hataları `load_json` üzerinden exit 1 (crash 5 değil); şema doğrulaması CLI'da `validate_full` (nested alan hatalarını da yakalar).
- **`status_summary`**: `Scheduler::status_summary()` türetilmiş-gerçek; `validate_document` A2'nin **beyan ettiği** `total`'ı görev sayısıyla çelişiyorsa `STATUS_SUMMARY_STALE` ile reddeder (yalan self-report geçmez).

**Dekoratif denetim sonucu (grep + SELECT/INSERT taraması):** koddaki tek `panic!` testi; hiçbir `allow(dead_code)` yok; `KV_BYTES_PER_TOKEN=60KiB` ve "~4 char/token" admission-heuristic'leri kaynak-notlu sabitler (sahte dönüş değil). 9 tablodan 4'ü (`runs/tasks/ledger/approvals`) yazılıyor; `model_usage/task_scores/prices/web_cache/archive_sections` **hiçbir okuma/yol tarafından dolu görünümlü kullanılmıyor** — §7.2'nin ileri-faz şeması, 0001 başlığına faz-faz sahipleri yazıldı (Faz 5/6/7/8). Yanıt üreten tek sahte-motor `MockExecutor` açıkça `"mock": true` işaretliyor.

Test: 110 unit + 6 budget + 8 golden + 13 integration = **137**, clippy `--all-targets -- -D warnings` 0, fmt temiz, skeleton 52/52; CLI karanlık-yol e2e: uncovered→1, bozuk-şema→1, compliance-kapalı→sonlu needs_user→2, gate-wait→4, deliver→0.

## Notlar / kararlar
1. `jsonschema` 0.18: `JSONSchema::compile` + `validate()` → `Err(iterator)`; `$id` sökümü şart.
2. Repair turunda violation mesajları DİNAMİK input'a eklenir; `context` ve `system_prefix` bayt-stable kalır (testli).
3. MockProvider, wiremock'a bağımlılık eklemeksizin aynı JSONL-senaryo yaklaşımını karşılar (dev-dep minimalizmi).
4. Toolchain allowlist'i şimdilik **tam vektör eşleşmesi**; araç tanımının izin verdiği bayraklar için desen (pattern) katmanı Faz 5'te (A5 görev hattı) gerekebilir.
5. Yerel KV boyutu sabit tahmin (60 KiB/token); Faz 8'de nvml probe'u gerçek boş VRAM'i verecek — `free_vram_mib` closure'u buna hazır tasarlandı.
6. EnvLayer fiyatlandırması şimdilik tek-olluk; routing altında local kolu $0, cloud kolu price-spec'li olacak — run-düzeyi `spent_usd` muhasebesi **Faz 4'te bağlandı** (soft/hard clamp + Paused), call-düzeyi reconcile EnvLayer'de. Faz 5 `model_usage` satırlarını per-provider doldurur.
7. **`tracing`** Cargo.toml'da; `tracing-subscriber`/`-serde` offline registry'de olmadığı için **kendi minimal `logging.rs` subscriber'ımız** (§15) kullanılıyor — WARN+ → stderr, events.jsonl'e dokunmaz.
8. **`final_digest` normalizasyonu** (Faz 4-c): hash = {phase, crisis, tasks, ledger-op DISTINCT-tuple'ları, olay SET'i(seq/ts/run_id/correlation soyulmuş, dedup'lu)}. Gerekçe §21.2 "aynı nihai hash": B4 penceresinde yeniden-yayınlanan olaylar idempotent toparlanmanın KANITIDIR — set normalizasyonu bunları tekleştirir; sıralı-dosya karşılaştırması (insta) Faz 5 rapor testlerinde ayrıca gelecek.
9. **Gate timeout ankoru**: `approvals`'daki `pending` satırının `ts`'i saat başlangıcı; poll'da ASLA yeniden-stamp edilmez (testli) — §9 Q4'ün "asla durdurmaz"ı böylece süreç-ötesi geçerli. Faz 5'te `GateRequested` semantik ayrımı netleşirse migration 0002 eklenebilir.
10. **TaskStarted → ledger.start'tan ÖNCE** yazılabilir (op_id deterministik türetilir, correlation geçerli): paralel grubun tamamı akışta "başlamış" görünür. Kabul edilmiş minimal sapma — olay zarfı "hangi işlemden doğdu" sorusunu op_id ile yanıtlamaya devam eder.
11. **CLI mini-run** `--tasks <tasks.json> [--spec <spec.json>]` bekler; `run` komutu varsa resume, yoksa create. `--spec` verilirse §6.4 kapısı canlı; `--assume-compliant` FinalReview kararını sürücüye veren bayraktır. Run-düzeyi bütçe kelepçesi bağlı (soft/hard + Paused). **Faz 5-c'ten beri**: A1'in gerçek `spec_match_pct` hükmü `advance_with` + `AgentPipeline` hattından gelir (kütüphane + e2e kanıtlı); CLI'nin `--mode interactive` sarmalayıcısı (interview/kapı yaşam-döngüsü) hâlâ açık iş — mini yol bilinçli olarak `advance` bool sarmalayıcısında kaldı.
12. **A1/A2/A3 = kontrol-düzlemi şapkaları, DAG görevi değil**: `registry()` yalnız yürütücü işçileri taşır (A5/A7/A4); planlama ajanları sürücü faz sınırlarında explicit metodlarla çağrılır (`FinalPipeline` dikişi). A2 bir planı bunlara atarsa plan REDDEDİLİR (erken başarısızlık, dispatch-gecikmesi değil). Denetlenmemiş varsayımın ihlal sayılması bilinçli konservatif kural: sessizlik asla yeşil ışık olamaz (§5.4C).
