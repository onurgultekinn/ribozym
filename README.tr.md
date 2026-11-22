# ribozym

**Tek cümlelik bir fikirdi çalışan bir yazılıma dönüştüren, otonom çok-ajanlı Ar-Ge ekosistemi.**
Tek kaynak gerçeklik: `agent-sistem-planlama-v2.md` (v2.47). Bu depo o planın Rust çekirdeğidir.

```
kullanıcı formu → A1 mimar (spec + ADR kararları) → [spec kapısı: onay sizde]
→ A2 CEO (görev DAG'ı) → [plan kapısı] → A5 kodlar → A7 tarar → A4 test yazar
→ A3 raporu dizer (sayılar koddan) → A1 hakem (spec_match %100 değilse teslim YOK)
→ [teslim kapısı] → Delivered
```

---

## Kurulum

```bash
git clone <repo> && cd ribozym
cargo build --release          # tek binary: target/release/ribozym
export OPENROUTER_API_KEY=...  # bulut birincil motor (Q1 kararı)
```

Yerel GPU motora (llama-server) isterseniz `LLAMA_SERVER_URL`设 edin ve
`config/models.toml [local].eligible_task_types` listesi ayak işlerini yere çeker
(bütçe/VRAM kelepçesi §10.1).

## 5 dakikalık akış

```bash
# 1) boş formu üret ve doldur (bölümler + `run_budget_usd` zorunlu — §5.4)
ribozym new-form --out spec_form.md
$EDITOR spec_form.md

# 2) uçtan uca koştur (gerçek LLM'ler, gerçek cargo; izole workdir'e yazar)
ribozym run --mode interactive --form spec_form.md --workdir ./wd --yes

# 3) takılan kapı / durak varsa:
ribozym status                    # her run: faz, harcama, açık kapı
ribozym approve <spec-id> spec    # kapı oyu (spec|task_plan|delivery)
ribozym run --mode interactive --form spec_form.md --workdir ./wd   # resume

# 4) çıktıları oku — hepsi TÜRETİLMİŞ GERÇEK, koddan:
ribozym report <spec-id> --format md
ribozym budget show
```

Mini (deterministik, LLM'siz) hat: `ribozym run --mode mini --tasks plan.json --yes`.

## Çıkış kodları (§20.2)

| kod | anlam |
|-----|--------|
| 0 | Delivered |
| 1 | kullanıcı hatası (argman/dosya/config) |
| 2 | NeedsUser / run başarısız |
| 3 | BUDGET — bütçe duvarı (duvarı yükseltip resume edin) |
| 4 | kapı onayı bekleniyor |
| 5 | iç hata (bug — Issues'a düşürün) |

## Çalıştırmanın anatomisi (her şey diskte, denetlenebilir)

```
.ribozym/state.db        tek gerçeklik kaynağı: runs/tasks/ledger/approvals/web_cache
runs/<spec>/raw_input.md formun ta kendisi (provenance sha256 ile kilitli)
runs/<spec>/spec.json    A1 taslağı (A1 kodu: hash'i makine yazar, model değil)
runs/<spec>/plan.json    A2 planı (revizyonlar append: 003-plan, 005-plan…)
runs/<spec>/report.json  A3'ün MÜHÜRLÜ raporu — sayılar SQLite/olaylardan
runs/<spec>/compliance.json  A1 hakem hükmü (pct kodun yeniden hesabı)
runs/<spec>/events.jsonl append-only olay akışı (seq monotonik, fsync)
runs/<spec>/archive/     §7.6 arşiv: bölümler + manifest sha256; okumada doğrulama
```

**Durma-devam sözleşmesi (§7.4):** süreç her an öldürülebilir; resume, ledger'daki
`started` operasyonları kapatır — sonuç dosyada ise model yeniden ÇAĞRILMAZ
(exactly-once fatura), değilse aynı girdiyle tekrar koşar. Bütçe şarjı `charged_at`
muhafızıyla tam-bir-kezdir; kill→resume aynı final hash'i üretir (golden testli).

## Bütçe güvenliği (§8.2)

- **Koşturmadan önce:** her görevin BEYAN edilen tahmini sert duvara çarpıyorsa
  görev başlamaz → `Paused(budget)`.
- **Çağrı sınırında:** her LLM turu EnvLayer'den geçer; kalan bütçe yetmiyorsa
  çağrı yapılmaz (para duvarı görevi başaramaz sayılmaz → Paused).
- **Yüzde-80:** tek `BudgetWarning` olayı (latch yapışkandır, yeniden başlamaz).
- Bütçeyi yalnız **siz** değiştirirsiniz (ribozym.toml / spec).

## Modelin yalanını kod nasıl tutar?

- Şema-zorunlu turlar + **tek** onarım turu (§14); ikinci bozukta MODEL_SCHEMA_INVALID.
- Araçlar yalnız allowlist'li ToolGateway'den (tam-argv eşleşme, scrub'lı ortam).
- A5 `tests/` yazamaz (A4'ün mülkü); A4'ün testi AC-id cite etmezse reddedilir.
- Hakem puanı modelin iddiasından değil, statülerinden **kod tarafından**
  yeniden hesaplanır; %100 altı asla "approved" olamaz; denetlenmemiş varsayım
  ihlal sayılır (§5.4C).
- Yönetim hatları da faturalı ve ledger-korumalıdır (plan_turn'lar).
- Web içeriği yalnız A11 kapısından; snippet'ler HASH olarak kaydedilir,
  metin asla prompt'a akmaz; talimat-özellikli içerik `injection_flagged` → low.

## Testler ve kapılar

```bash
bash scripts/gates.sh     # cargo fmt --check + clippy -D warnings + cargo test
                          # + python scripts/validate_skeleton.py (55 kontrol)
```

Testlerde LLM asla canlı çağrılmaz (§21.1, mock script); `cargo check/test`
gerçek koşar; goldenlar kill→resume'un aynı hash'i ürettiğini kanıtlar (§21.2).

## Dil ve dürüstlük kuralları

- `AGENTS.md`: kod/JSON/şema/log/commit **İngilizce**; yorum ve kullanıcı metni **Türkçe**.
- `PROGRESS.md` gelişim defteridir: her faz, her bulunan gerçek hata ve onu
  kilitleyen test orada. "Dekoratif yeşil" yok: yazılmayan/okunmayan kolon
  yok sayılır ve faz sahipliği migration başlıklarında yazar.

## Şu an açık olan (plan v2.47'ye göre)

`interview` soru-cevap akışı, `suggest` kanalı (§5.5), A6/A8/A9/A10 ajanları,
web'in LLM kör-puan katmanı (v1.0), `calibrate`, zstd sıkıştırma (dep yok).
Detay: PROGRESS.md "Sıradaki — Faz 6/7/8".
