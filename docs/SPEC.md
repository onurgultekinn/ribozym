# OTONOM ÇOK AJANLI AR-GE EKOSİSTEMİ — PLANLAMA BELGESİ v2.0

> **Belge kontrolü**
>
> | Alan | Değer |
> | --- | --- |
> | Version | 2.47 (v2.0 + karar kaydı + `ribozym` + A1-A12 kilitli şartnameleri + iletişim zarfı + arşiv katmanı + kart etkileşimi + çıktı ekonomisi; v2.40: A9 = Release & Distribution; v2.41-44: A10 = Bütçe Planlayıcı & Model Tedarikçisi + denge kuralı; v2.45: doğrulama → A11 verify şapkası; v2.46: Arşiv & Kurye → A12, kadro A1-A12 bitişik; v1 arşivdedir, bu dosya tek kaynaktır) |
> | Status | Pre-code, final planlama dokümanı |
> | Language policy | Açıklamalar Türkçe; **kod, JSON şemaları, alan adları, log/commit mesajları, prompt'lar, enum değerleri tamamen İngilizce** |
> | Time policy | Tüm timestamp'ler UTC, ISO-8601 (`Z` sonekli) |
> | ID policy | Tüm kimlikler küçük harf + tire: `spec-20260911-001`, `T1`, `A5`, `evt-...` |
>
> **v1 → v2 diff özeti (neden yeniden yazıldı):**
>
> 1. Lineer durum makinesi ile `tasks.json`'daki paralel/bağımlı görev modeli çelişiyordu → **iki katmanlı orkestrasyon** getirildi (§6).
> 2. `state.db` hem `runs/` altında hem `.ribozym/` altında tanımlıydı → **tek global state store** kararı (§7).
> 3. `agents.toml`, onay protokolü, bütçe hiyerarşisi, A11/A12 girdi/çıktı sözleşmeleri tanımsızdı → tanımlandı (§9, §10, §13, §16).
> 4. Resume mekanizması idempotency'sizdi → **operation ledger + event seq/correlation** modeli (§7.4).
> 5. `sysinfo` crate'i VRAM okuyamaz (GPU telemetrisi yok) → teknik yığın düzeltildi (§4).
> 6. Model seçimi düz yazıydı → **skor formülü + pseudocode** (§10.3).
> 7. Fazlarda çıkış kriteri, efor tahmini ve MVP kesim çizgisi yoktu → eklendi (§22).
> 8. Risk kaydı, test stratejisi, izlenebilirlik (requirement↔task↔acceptance) kuralları yoktu → eklendi (§21, §23, §6.4).
> 9. v1'deki format bozuklukları (birleşmiş satırlar, kaçalı linkler) temizlendi.
> 10. **v2.1:** §25'teki 6 açık soru kullanıcı kararıyla kapandı; ilgili bölümlere işlendi (§9, §10.1, §12.6, §13, §21.3, §22).
> 11. **v2.2:** Ürün adı kullanıcı kararıyla `ribozym` (crates.io + GitHub görünürlük analiziyle seçildi; bkz. §26); repo kökü `C:\Users\Rog\Desktop\ribozym\`.
> 12. **v2.3:** Çıkarım mimarisi düzeltildi — **bulut API birincil motor; yerel model yalnız yardımcı offload** (v1/v2.0'da yanlışlıkla "önce yerel" kurgusu vardı). Bkz. §10.
> 13. **v2.4:** A1 giriş zayıflığı kapatıldı — context pack + clarify turu + `assumptions[]` sessiz tahmin yasağı + A4 test-edilebilirlik damgası (§5.4, §6.1, §6.4, §11.1).
> 14. **v2.5:** A1 girişi üç aşamalı oldu — kullanıcı **formu** (`spec_form.md`) → A1 **araştırma** fazı (prior-art/rakip/risk) → **tavsiye diyaloğu** (kullanıcı önerileri kabul/ret eder; kararlar + bütçe + MVP kilitlenir). FSM'e `Clarifying` ve `Advising` durumları eklendi.
> 15. **v2.6:** A1, **Chief Architect & Referee** olarak yeniden konumlandırıldı — teknoloji kararları dahil tüm proje kararlarının sahibi; `decisions[]` ADR formatında zorunlu alanlarla; A5 sapması yalnız `SpecRevised` ile; FinalReview'da deterministik decision-compliance denetimi (§5.1, §5.4, §11.1, §17).
> 16. **v2.7:** **Kullanıcı öneri kanalı (What-If loop)** eklendi — kullanıcı run boyunca fikir önerebilir; her öneri etki analizinden geçer (mantıklıysa entegre, pahalıysa onaylı, değilse alternatifli ret); §24'e kural 11 eklendi (§5.5, §20.1, Ek A).
> 17. **v2.8:** Öneri kanalı **danışma modeline** çevrildi — kullanıcı belirler, AI analiz eder ve cevabını verir, **kullanıcı onaylarsa eklenir**; onaysız entegrasyon kaldırıldı (v2.7'deki otomatik-entegrasyon yolu).
> 18. **v2.9:** Giriş akışı yeniden sıralandı (kullanıcı kararı): **soru-cevap (ajansız/ucuz, hazır soru bankası) → ajan analizi + açıklama (dolu form tek çağrıda) → düzeltme dönemi (iteratif diyalog) → kilit**. Token tasarrufu ilkesi: soru bankası statik dosya, cevap analizi Aşama 1'de yok, tam belge tek çağrıda okunur (§5.4, §19 config/questions/, §20.1 `interview`).
> 19. **v2.10:** Aşama 1 soruları hard-coded olmaktan çıktı — **hibrit adaptif model**: statik çekirdek set + cheap-tier soru-uyarlama ajanı fikre özel sorular üretir; turlar cevaplara göre dinamik ayarlanır (max 3 tur).
> 20. **v2.11:** (kullanıcı kararı ile v2.10'un uyarlaması) Soruların **tamamı genel** — bir projeyi planlarken sorulan evrensel sorular; ajan yeni soru üretmez, yalnızca her genel soruyu **anlık bağlama uyarlar**. Soru içeriği standart, hiçbir kritik başlık atlanmaz.
> 21. **v2.12:** A1 görev tanımı analiz edildi ve tier eşlemesi kesinleştirildi — **§5.4A model-tier haritası** eklendi (aşama bazlı cheap/strong); soru-uyarlamanın ayrı ajan değil A1'in cheap modu olduğu açıkça yazıldı; §5.1'deki "Local (giriş)" v2.3 bulut-birincil kararıyla uyumlu hale getirildi.
> 22. **v2.13:** **Karar kartları (§5.4B)** — teknoloji kararları serbest sohbet yerine kart ekranıyla açıklanır/onaylanır: kartlar karar kilidi çağrısının aynısında üretilir (ekstra çağrı yok), görüntüleme 0 token, sorgulama dar kapsamlı mini çağrı, `değiştir` ADR revizyonu tetikler. `ribozym review` komutu eklendi.
> 23. **v2.14:** **Çıktı ekonomisi ilkesi (§14.1)** — "input ucuz, output pahalı": tek üretim çok kullanım; yapılandırılmış kısa çıktı varsayılan; tekrarlayan kalıplar deterministik şablon; öngörülebilir içerik aynı turla üretilir; büyük girdi + küçük çıktı tüm prompt'ların son talimatı. §24'e kural 12 eklendi.
> 24. **v2.15:** §14.1'e **asimetrik iletişim** (kural 6: kullanıcı serbest yazar — input ucuz; AI her zaman kısa ve sabit formatta) ve **tek format ailesi** (kural 7: tüm planlama çıktıları aynı kart şablonu) eklendi; Aşama 2 açıklaması kart formatına bağlandı.
> 25. **v2.16:** (kullanıcı eleştirisi) §5.4B onay grameri (`ok D1 D3` kod dili) kaldırıldı → **basit Evet/Hayır soruları + serbest yazı**; §5.5'e "run bozulmaz garantisi" eklendi (analiz paralel, entegrasyon görev sınırında, iptaller `skipped`, resume-tutarlı).
> 26. **v2.17:** **Depo-önce ilkesi** — "Hayır"ın ilk cevabı LLM'sizdir: kartın genişletilmiş detayı aynı üretim çağrısında depolanır, yönlendirme metinleri koddur; LLM yalnız depoda olmayan yeni içerik için çalışır (§5.4B, §14.1 kural 8). §5.5'e **düzenleme sınıfları** eklendi: tamamlanmış işe dokunan öneri ağır yoldan (deterministik etki listesi + maliyet + regression + erteleme seçeneği), kilitli karara dokunan yeni ADR + run sonrası değerlendirme.
> 27. **v2.18:** (kullanıcı düzeltmesi — v2.17'nin Hayır akışı yanlış kurulmuştu) Doğru akış: **Hayır seçilince deterministik açıklama alanı açılır** ("neden olmadı, ne değişsin? serbest yaz") → kullanıcının açıklaması LLM'e girdi olur → düzenleme yapılır → sonuç **kısa kart + Evet/Hayır** olarak yeniden sunulur. LLM yalnız düzenleme turunda çalışır.
> 28. **v2.19:** **suggest akışı tek biçime indirildi** — her öneri aynı yoldan: A1 etki analizi (paralel) → **tek kart**: etkilenen kısımlar + tahmini süre + tahmini maliyet → **Evet/Hayır**. Bitmiş işe dokunan öneri aynı kartta görünür; Evet'te o görevler regression ile yeniden koşar. "Uygulanamaz" kartında Evet yoktur: Vazgeç ya da "Yine de isterim" (ağır yol).
> 29. **v2.20:** (kullanıcı düzeltmesi) suggest de karar kartlarıyla **aynı döngüye** bağlandı — sistemde tek etkileşim kalıbı: kart → Evet/Hayır → Hayırsa açıklama alanı → düzelt → tekrar sor. Hayır'da öneri kapanmaz, açıklamaya göre revize edilir; vazgeçme her an mümkün.
> 30. **v2.21:** **A1 TAM ŞARTNESİ konsolide edildi ve KİLİTLENDİ (§5.4C)** — v2.4–v2.20'nin 15 iterasyonu tek bağlayıcı tanımda birleştirildi (7 görevlik zincir + 5 kesin kısıt). Çatışma durumunda §5.4C kazanır; değişiklik yalnız açık kullanıcı kararıyla.
> 31. **v2.22:** **Arşiv katmanı (§7.6)** — her aşama çıktısı zstd ile sıkıştırılarak `runs/<spec_id>/archive/` altına koddan arşivlenir (arka plan, düşük donanım); aşamalar bağlamını arşivden alır (çelişki önleme = Risk 1 çözümü); resume'un tam içerik yedeği; `.gitignore`'da, GitHub'a gitmez. §5.4C'ye kısıt 6 eklendi.
> 32. **v2.23:** (kullanıcı kararı) **Model seçimi çağrı-bazlı oldu** — aşama-tier kilidi kalktı; her LLM çağrısı `task_type` ile selector'a gider, `task_scores.db` geçmişine göre bütçe içinde en iyi model seçilir (Risk 3 çözümü). §5.4A haritası başlangıç önerisine indirildi; §10.3'e çağrı-bazlı seçim kuralı eklendi. (Risk 2 — prompt kalitesi — zamanla test-geri-bildirimle olgunlaşacak, Faz 5 golden testleri ölçer.)
> 33. **v2.24:** **A2 (CEO) tasarımı kabul edildi (§5.6)** — plan kartı + tek etkileşim kalıbı; deterministik tahmin (task_scores.db + A10, LLM tahmini değil); replan iki katman (kapsam-içi otonom / kapsam-değişikliği suggest + onay); granüllük kuralı (tek ajan, tek dosya, tek oturum, ≥1 gereksinim).
> 34. **v2.25:** (kullanıcı kararı) **A2'nin çerçevesi netleşti: A1 teorik, A2 pratik** — A1'in sonucunu en verimli task'lara böler (ne çok küçük ne çok büyük; ölçüt task_scores.db) ve departmanlara dağıtır. **Rapor-temelli sistem denetimi eklendi:** A3'ün birleştirilmiş departman raporlarını A2 analiz eder, aksayan kısımları (gecikme, tekrarlayan hata, bütçe sapması) kart olarak kullanıcıya bildirir; canlı müdahale değil, rapor üzerinden denetim.
> 35. **v2.26:** (kullanıcı düzeltmesi) Aksayan kısımlar **kullanıcıya değil, sorumluya gider** — sistem öz-düzeltme yapar: görev hatası → fix döngüsü, bütçe sapması → A10, model hatası → selector, gecikme → scheduler. Kullanıcıya yalnız bilgi kartı ("şu aksıyordu, şöyle çözüldü"); yetki aşımı (kapsam değişikliği, E4) için suggest/NeedsUser.
> 36. **v2.27:** (kullanıcı kararı) **Aksama tanımı genişletildi:** öncelikli ölçüt "yeterli verimde çıktı verememe" — kalite/verim düşüşü raporlarda öne çıkıyorsa sistem verimi korumak için düzeltmeye çalışır; tespit koddan sabit eşiklerle (zamanla ayarlanır). **Sağlık kartı:** küçük öz-düzeltmeler run sonunda tek özet kart; önemli olaylar anında kart. (Replan maliyet çizgisi: replanın tahmini ek maliyeti küçükse — varsayılan %10, config — otonom; büyükse etki kartıyla onaylı. Değişiklik verimliliği: replan/düzeltme yine API ile ama en verimli şekilde — aynı sonucu en az yeni görevle, örn. 4 yerine 2 görev; gereksiz görev yok, amaç az çağrıyla verimi korumak.)
> 37. **v2.28:** (kullanıcı kararı) **Replan maliyet çizgisi kabul edildi** — replanın tahmini ek maliyeti küçükse (varsayılan %10, config) A2 otonom uygular, büyükse etki kartıyla onay ister. **Değişiklik verimliliği:** replan/düzeltme yine API ile ama en verimli şekilde — aynı sonucu en az yeni görevle (4 yerine 2 görev gibi); gereksiz görev yok, amaç az çağrıyla verimi korumak.
> 38. **v2.29:** **A2 TAM ŞARTNESİ konsolide edildi ve KİLİTLENDİ (§5.6A)** — 8 görevlik zincir + 5 kesin kısıt. **A1 + A2 = Yönetim Departmanı** olarak resmileşti (A1 tanımlar/teorik, A2 pratikleştirir/yönetir); kalan 10 ajan icra/destek departmanları.
> 39. **v2.30:** (kullanıcı kararı) **A13 Archive Agent kadroya girdi** (deterministik, modelsiz): departman çıktılarını çekip arşive sıkıştırır (§7.6); **A3'ün veri kaynağı**. **A3 tasarımı (§5.7):** rapor = deterministik iskelet + LLM anlatı (sayıları tekrar edemez); ara rapor ritmi tanımlandı; final rapor A1 onayıyla dondurulur; tüm raporlama tek yetkide. **§11.9 iletişim zarfı + §24 kural 13:** tüm ajan-ajansı iletişim JSON zarfla Rust çekirdeğinden — deterministik alanlar koddan, LLM yalnız üretilmesi gereken payload alanlarını yazar.
> 40. **v2.31:** (kullanıcı düzeltmeleri) **A3'ün LLM rolü = veri analitiği** (yorum değil): sayılardan anlamlı sonuç çıkarır; sistem içi raporlarda yorum katmaz, sistem dışı raporlarda çıktı-verimli açıklayıcı sunar. **A13 = Arşiv & Kurye:** tüm sistemi gezer, log kalitesinin gözcüsüdür (log ne kadar iyiyse sistem o kadar iyi çalışır); **denge kuralı:** yalnız deterministik iyi yapabildiklerini kendisi yapar, yapamadığında API'ye başvurur (§5.8).
> 41. **v2.32:** **A3 ve A13 KİLİTLENDİ (§5.7, §5.8 başlıkları)** + A3'ün rapor alanları sınırları resmileşti: final / ara / sağlık kartı içeriği = rapor; canlı sorgu görünümleri, gate kartları, ham artifact'ler = rapor değil (§5.7 m6).
> 42. **v2.33:** (kullanıcı kurgusu) **A4 = Test Arşivi Mimarı (§5.9):** `tests/` klasörü yalnız A4'ün mülkiyeti (A5 test yazamaz); **canlı arşiv** — güncelleme, onarım, emeklileştirme, yeni test üretimi; sistem-bazlı büyük benchmarklar; **hakkaniyet garantisi** (AC'ye bağlama + bağımsız test-değişikliği kaydı + A2 denetimi); doğrulanmış hata listesi; etkilenen-alan koşumu + flaky yönetimi + kapsam eşiği.
> 43. **v2.34:** (kullanıcı düzeltmesi) A4 = **test kurgulayıcı** — oluşturma, güncel tutma, testlerdeki hataları çözme; "hata listesi" çıktısı şimdilik görevinden çıkarıldı (koşum kaynaklı hatalar pipeline akışına ait, §6.3). **Yazım/koşum ayrımı netleşti:** test yazımında model zorluğa göre seçilir (yerel/güçlü), koşum her zaman yerel makinede.
> 44. **v2.35:** (kullanıcı kararı) **A5 = Lead Coder sadeliğiyle KİLİTLENDİ (§5.10):** tek işi projeyi kodlamak; disiplinini sistem sınırlarından alır — ADR çerçevesi, dosya sahipliği (`tests/` yasak), fix döngüsü, çağrı-bazlı model seçimi, genel ajan kuralları. Ekstra mekanizma yok.
> 45. **v2.36:** (kullanıcı kurgusu) **A6 yeniden doğdu: R&D Laboratuvarı (§5.11)** — A1'in araştırmasını yapmaz (A1, A11 verisiyle kendisi yapar; §5.4/§5.4C güncellendi); bilimsel akışla hipotez üretir → A4 deney testi → A7 izole koşum → başarılı öneriler A2'ye (suggest + kullanıcı onayı); keşif kotasından beslenir, arka planda çalışır. **A7 = Sandbox & Security (§5.12):** her türlü izole ortam işinin sahibi + antivirüs gibi sürekli güvenlik taraması. **v2.36 uzatması:** A6+A7 şartnameleri kullanıcı onayıyla KİLİTLENDİ (başlıklar güncellendi).
> 46. **v2.37:** **A8 = Technical Writer KİLİTLENDİ (§5.13):** tek işi proje dokümantasyonu (README, API doc, CHANGELOG); **İngilizce + kısa ve öz** — ne aşırı kısa ne aşırı uzun (kullanıcı kararı); içerik gerçekliği yansıtır (komutlar A7 izole ortamında doğrulanır); öz değişmedikçe yeniden yazım yok, bölüm güncellemesi var.
> 47. **v2.38:** (kullanıcı kararı) **A8 genişletildi: Technical Writer & Repo Manager** — GitHub yönetimi de A8'de: commit disiplini (Conventional Commits, başarısız çıktı asla commit edilmez, mesajlar şablonla), push politikası (yalnız teslimat onayı sonrası, `ribozym/<spec_id>` branch, `main` asla), repo düzeni. A9 altyapı **üretimiyle** sınırlandı (Dockerfile/CI şablonu); yayımlama A8'de — dışarıya açılan kapı tek elde.
> 48. **v2.39:** (kullanıcı kararı) **A9 = DevOps KİLİTLENDİ (§5.14), sadelikle:** tek işi üretilen projeye dağıtım dosyaları üretmek (Dockerfile + CI şablonu); izole ortam işi zaten A7'de (karışmaz); ürettiği dosyalar A7 izolinde bir kez koşularak doğrulanır; yayımlama yetkisi yok (A8'e işler).
> 49. **v2.40:** (kullanıcı kararı) **A9 yeniden tanımlandı ve yeniden kilitlendi: Release & Distribution (§5.14)** — "dağıtım dosyası üreticisi"nden *"projenin son kullanıcı tarafından kurulabilir-çalıştırılabilir olmasının sahibi"*ne: çıktı yüzeyi Dockerfile/CI'nın yanında **GitHub Release workflow, installer (install.sh/ps1 + checksum) ve doğrulanmış kurulum komutları** — §5.13 m2'nin sahipsiz boşluğu (kurulum komutunu kim üretip doğruluyor?) kapandı. **Determinizm–API denge kuralı (A13 m4 ile aynı ilke):** varsayılan şablon + `decisions[]` (0 token); şablonun yetmediği işte selector'dan cheap API (A10 kelepçeli). Sınırlar duruyor: üretir-yayınlatmaz (A8), koşurmaz (A7), karar vermez (A1). Kayıt yayını (crates.io/Homebrew/winget) kapsam dışı → v1.x adayı (§1.2). Uğradığı değişiklikler: §5.1 kadro, §5.3 deterministik-ajan satırı, §11.1 `model_usage`, §3.2 akış. Eşlik eden düzeltme: §12.5'teki v2.38 kalıntısı push atfı `(A9)` → `(A8)`; başlıktaki havada kalan "Integration" adı kaldırdı.
> 50. **v2.41:** (kullanıcı kararı) **A10 yeniden doğdu: Bütçe Planlayıcı & Model Tedarikçisi (§5.15, KİLİTLİ)** — pasif harç-kelepçesi değil aktif planlayıcı: bütçe çerçevesi içinde iş + test kotası yönetimi, zorluğa göre havuzdan seçim, **A4'ten benchmark yazdırıp A7 izolinde kendi görev-bazlı tier list'ini üretir**; canlı run verisi tier list'i sürekli günceller. **Bootstrap:** 13 gün sınırsız 6-API erişimiyle `ribozym calibrate` havuzu baştan doldurur (bitiş ≈ 2026-09-25). §10.3 kendi-benchmark birincilliye çevrildi; keşif adlandırması ayrıldı (`model_test_quota` / `research_quota_pct`); §7.2'ye `task_scores` tablosu, §13.1'e teknik-bağlantı muafiyeti, §5.2'ye A10 hata modu (`PRICE_STALE` fallback) eklendi; §8.2 m2 fiyat-lehine kaymaya bağlandı.
> 51. **v2.42:** (kullanıcı düzeltmeleri) **A10'da üç nokta revize edildi (§5.15):** (1) **Bütçeyi kullanıcı dışarıdan tahsis eder** (aylık toplam + test kotası config'te) — A10 tahsis etmez, verilen çerçevede harcamayı yönetir. (2) **Zorluk etiketi A2'den gelmez** — A10 görevi kendisi analiz eder (metadata + `task_scores` geçmişi + DAG bağlamı, deterministik). (3) **Aday eleme OpenRouter API (liste+fiyat) + Artificial Analysis API (kalite) ile** — kriter: **ucuz ve performanslı**; eleme sonrası kendi benchmarkıyla test edip görev-bazlı tier list'e yerleştirir, seçim zorluğa göre tier list'ten yapılır. **Teslim planı sıkıştırıldı (§22):** planlama 2026-09-12'de tamamlandı; kodlama **≤3 gün** (faz tahminleri saat ölçeğine indirildi) — 13-günlük kalibrasyon penceresi bolca yeterli, v2.41'deki "pencere kaçarsa" senaryosu kaldırıldı; 7-full/8b v1.1'de kalır.
> 52. **v2.43:** (kullanıcı kararı) **Dış veri ritmi belirlendi (§8.2 m5, §10.3):** canlı fiyat **5 dakikada bir** arka planda OpenRouter'dan poll edilir, seçim cache'ten okur (çağrı-içi fiyat isteği yok); Artificial Analysis kalite skorları **günde 1 kez** çekilir. Seçim yolu dış API beklemez; poll başarısızlığında `PRICE_STALE` + %15 marj fallback'i (§5.2) geçerli kalır. R7 azaltım satırı güncellendi.
> 53. **v2.44:** (kullanıcı kararı) **A10'a determinizm–API denge kuralı eklendi (§5.15):** her işi o işi daha iyi yapan yapar — aritmetik/sıralama/eşik/kelepçe koddan; **zorluk analizi hibrit oldu (m2):** kural seti birinci yol (0 token), kural çıkmazında tek cheap-tier LLM sınıflandırması (girdi görev özeti, çıktı etiket+gerekçe; yol kaydı `rules`/`llm`). A10'un kendi çağrıları da model_usage'a yazılır. Etkilenen: §5.15 kimlik+kısıt 1, §5.1 kadro, §5.3, §11.1.
> 54. **v2.45:** (kullanıcı kararı) **Doğrulama ajanı kaldırıldı, A11'e birleştirildi: "Web Access & Verification" — iki şapka, iki model (§13):** fetch (agent-reach birincil) ayrı ajandı; doğrulama zaten yapısal olarak farklı model + kör girdi gerektirdiğinden ayrı ajan tekliği gereksizdi — dört-göz ilkesi veri yoluyla korunur (VerificationRequest'te üretici alanı yok), sahiplik tek elde. Verify, internet gerektirmez (yalnız finding verisi; tek internet kapısı bozulmaz). Etkilenen: §2 sözlük, §3.1/3.2 diyagramlar, §5 başlık+kadro+§5.2, §5.3 trait, §5.6 verim ölçütü, §5.15 kısıt 4, §10.3 kara liste, §10.4, §13 başlık+13.1+13.3, §21.2 prompt_optimizer, §22 fazlar, §24 kural 7, R9, Q2.
> 55. **v2.46:** (kullanıcı kararı) **Eski A13 (Arşiv & Kurye) → A12 yeniden numaralandı** — doğrulama ajanı kalkınca boşalan numara kadro bitişik olsun diye arşivciye verildi; kadro artık **A1–A12** (bitişik). İş tanımı, kilitli şartname (§5.8) ve tüm çapraz referanslar değişmedi; yalnız kimlik değişti. Geçmiş diff kayıtlarındaki "A13" ifadeleri tarihsel kalmıştır (o kayıtlar yazıldığı andaki kadroyu anlatır). **A6 incelemesi sonuçları (aynı sürümde):** §3.2'deki v2.36 öncesi kalıntı "(A6 research → context.md)" akıştan çıkarıldı (A1 araştırmasını A11'le kendisi yapar); m4'teki yanlış referans §13 → §5.15/config olarak düzeltildi; **m5 netleşti: A6 run dışında çalışmaz** (non-goal: daemon yok); **m6 eklendi: R&D hafızası = A12 arşivi** (hipotez/deney kayıtları arşive düşer, sonraki run'da tekrar denenmez). **Tüm-sistem taraması:** §12.6 brownfield'daki "A6+A5 tarz analizi" kalıntısı A5'e bağlandı; **context.md üreticisi netleşti** (§5.10 m1a: A1 Researching çıktısı — A5'in girdi boşluğu kapandı); §5.7'deki "13 departman" sayısı → 12.
> 56. **v2.47:** (kararlılık denetimi — tüm-sistem kararlılık analizi; kodlama öncesi 4 boşluk kapatıldı) **(1) FinalReview↔Crisis döngüsüne üst sınır (B1):** `FinalReview uyumsuz → Crisis → replan` geçişine sayaç eklendi — en fazla **2 tur**; tükenince karşılanamayan gereksinim(ler) kapsam kararı olarak kullanıcıya NeedsUser ile döner (kapsam küçültme kullanıcı kararıdır; sistem kendi kendini bütçeye kadar yakmaz). **(2) Arşiv bütünlüğü (B2):** §7.6 arşiv bölümleri sha256 manifest'le yazılır, okuma anında doğrulanır; bozuk bölümde sessiz-devam yasak — A12 işaretler, tam yeniden-yazım ya da NeedsUser; manifest `runs/<spec_id>/archive/manifest.json`'da tutulur. **(3) Bütçe duvarı → Paused(budget) (B3):** sert limitte doğrudan `Failed` yerine run `Paused(budget)` olur; kullanıcı bütçeyi yükseltip `resume` eder ya da suggest kartıyla kapsamı küçültür; %80 BudgetWarning kullanıcıya kart olarak da düşer; **fiyat-lehine kaymanın kalite takası** raporlara `quality_tradeoff` alanıyla yazılır (sessiz kalite düşüşü yasak). **(4) Ledger/olay yazım sırası (B4):** kesin kural — her işlemde sıra: `ledger INSERT started` → `state.db transaction` → `events.jsonl append` → `ledger UPDATE finished`; resume "olay var + ledger finished yok" penceresini §7.4 tür-bazlı kurtarma kurallarıyla kapatır; golden resume testine "olay-ledger ayrık çökme" varyantı eklendi. Etkilenen: §6.1, §7.4, §7.6, §8.2, §17, §21.2, §23 (R11, R12).

---

## 1. YÖNETİCİ ÖZETİ

**Sistem bir bakışta:** Kullanıcı proje fikrini metin olarak verir. Sistem bu fikri önce doğrulanmış bir şartnameye (`specs.json`) çevirir; şartnameyi görevlere böler, görevleri 12 uzman ajana dağıtır ve **bulut LLM (birincil motor) + yerel LLM (yardımcı hızlandırıcı)** ile OS/CLI araçlarını kullanarak baştan sona bir yazılım projesi üretir. Üretilen proje çift imzalı denetimden geçer ve teslim edilir. Her adım kayda geçer: sistem çökerse kaldığı yerden devam edebilir, harcanan her kuruş izlenebilir.

**Ürün:** Rust ile yazılmış `ribozym` — paket adı ve çalıştırılabilir dosya adı aynı.
**Mimari ilke:** API-önce çekirdek + olay güdümlü kontrol düzlemi + tek yazarlı SQLite durum deposu.

### 1.1 Temel ilkeler (her biri test edilebilir — §24'te kesin kurallar olarak tekrarlanır)

| # | İlke | Test kanıtı |
| --- | --- | --- |
| P1 | API-önce çekirdek | `lib.rs`'in herkese açık fonksiyonları CLI'a bağımsız çalışır; CLI yalnızca bunları saran ince bir katmandır |
| P2 | Olay güdümlü | Her durum değişikliği `events.jsonl` içinde artan `seq` numarasıyla bulunur |
| P3 | Devam etme (resume) | Sistem ortadan öldürüldükten sonra `ribozym resume` kaldığı yerden devam eder; bitmiş görevi tekrar **çalıştırmaz** |
| P4 | Önce ucuz | Varsayılan yol: deterministik kod, sonra en ucuz uygun bulut modeli; yerel model maliyeti düşürür; hangi kademenin kullanıldığı `escalation_level` alanında görünür |
| P5 | Çift imza | `Delivered` olayı yalnızca A1 `spec_match_pct == 100` bildirirse yayımlanır |
| P6 | Bütçe bilinci | Her model çağrısından önce ve sonra maliyet kontrolü yapılır; limit aşılırsa bulut yerine yerel model seçilir ya da run durdurulur |
| P7 | Güven katmanı | Kodun içinde gizli anahtar yoktur; izin listesi dışında komut çalışmaz; internetten gelen içerik "veridir, talimat değildir" |

### 1.2 Kapsam dışı (NON-GOALS — v2'de açıkça yazıldı)

Bunlar bilinçli olarak MVP ve v1.0'da **yok**; mimariyi bozmadan sonradan eklenebilir:

- Web / WebSocket / mobil arayüz (yalnızca programatik API yüzeyi hazır olacak).
- Aynı anda birden çok `run` (aynı anda tek spec işlenir; kuyruk yok).
- Kubernetes/Terraform/üretim-dağıtım otomasyonu (A9 yalnız dağıtım yüzeyi üretir: Dockerfile, CI, release workflow, installer — §5.14).
- Otomatik GitHub birleştirme/PR onayı (sistem push atar ve PR açar; birleştirme kararı kullanıcıya aittir).
- Kendi kendini yeniden yazan ajanlar / meta-öğrenme.
- Run dışı kalıcı ajan çalışması (daemon): A6 dahil tüm ajanlar yalnız run süresince yaşar; R&D sürekliliği arşiv üzerinden sağlanır (§5.11 m5-6).
- Dil desteği: MVP'de yalnızca **Rust toolchain** (Python/TypeScript desteği v1.1'e ertelendi — bkz. §22 kesim çizgisi).

---

## 2. TERMİNOLOJİ (sözlükçe)

| Terim | Anlamı |
| --- | --- |
| Spec | A1'in ürettiği, imzalanmış bağlayıcı şartname (`specs.json`) |
| Run | Bir spec'in baştan sona bir kez işlenmesi; `run_id` ile kimliklidir |
| Task | Görev ağacındaki (DAG) bir iş birimi (`T1`...); bir ajan tarafından yürütülür |
| Gate | İnsan onayı gereken kontrol noktası (`spec` / `task_plan` / `delivery`) |
| Ledger | `state.db` içindeki işlem defteri; yarım kalan işlemlerin kaydı |
| Blind scoring | A11'in doğrulama şapkasının, bir bilgiyi kimin ürettiğini görmeden puanlaması (§13.3) |
| Escalation ladder | 0–4 arası kademeli hata tırmanma protokolü (§8) |
| Golden run | Sahte (mock) LLM'lerle çalışan uçtan uca referans çalıştırma; çıktısı önceden kaydedilen sonuçla karşılaştırılır (§21.3) |
| Tier list | A10'un görev türüne göre yaptığı model sıralaması — kendi benchmarkları + gerçek run verisinden (§5.15, §10.3) |

---

## 3. MİMARİ GENEL BAKIŞ

### 3.1 İki düzlem (control plane / data plane)

```
CONTROL PLANE (deterministik kod — LLM yok)
├── Orchestrator FSM        spec-lifecycle durum makinesi (§6.1)
├── Task Scheduler          DAG çalıştırıcı, worker pool, paralel gruplar (§6.2)
├── State Store (SQLite)    runs / tasks / ledger / approvals / usage (§7.2)
├── Event Bus → events.jsonl (append-only, seq'li) (§7.3)
└── Policy Engine           bütçe + sandbox + allowlist + otonomi (tek kapıdan geçer)

DATA PLANE (LLM destekli, deterministik olmayan)
├── Inference Layer         local.rs (llama-server) / cloud.rs (OpenRouter) (§10)
├── Agents (12 ajan, §5.1)  her biri: girdi sözleşmesi → LLM/araç → çıktı sözleşmesi
└── Tools Layer             cargo/git/docker/os/web (§12) — hepsi Policy Engine'den geçer
```

**Kural:** LLM çıktısı hiçbir zaman doğrudan iş yapmaz. LLM yalnızca **önerir** (kod, görev planı, karar JSON'u); işe dönüşen her şey kontrol düzleminin sabit doğrulamasından geçmek zorundadır (şema kontrolü, izin listesi, bütçe, izole ortam).

### 3.2 Bir run'ın veri akışı

```
spec_form.md ─A1 (form → araştırma → tavsiye diyaloğu, §5.4)→ specs.json ─[gate:spec]→ A2 → tasks.json ─[gate:task_plan]→
Scheduler → A5 code → A7 security ⇄ A4 test →
A9 dağıtım yüzeyi → A8 docs+git → A3 report → A1 final review ─[gate:delivery]→ Delivered (push A8'den, onaydan sonra)
                ↑ A11 web + kör doğrulama (ikinci model; herhangi ajan tetikleyebilir)
                ← A10 bütçe/model politikası tüm cloud çağrılarına kelepçeli
```

### 3.3 Herkese açık API yüzeyi (P1 kanıtı)

`ribozym` hem kütüphane hem program olan (lib + bin) bir Rust crate'idir. Herkese açık yüzeyi:

```rust
// src/lib.rs
pub fn run_spec(input: SpecInput, opts: RunOptions) -> RunHandle;  // async tokio
pub trait RunHandle {
    async fn events(&self) -> mpsc::Receiver<Event>;   // streaming (ileride WS köprüsü)
    async fn approve(&self, gate: Gate) -> Result<()>; // programatik onay
    fn snapshot(&self) -> RunSnapshot;                 // status/report veri kaynağı
}
pub fn resume(spec_id: &str) -> Result<RunHandle>;
pub fn load_report(spec_id: &str) -> Result<StandardDepartmentReport>;
```

CLI bu yüzeyin ince sarmalayıcısıdır; gelecekteki Web UI aynı yüzeyi kullanır, çekirdek yeniden yazılmaz.

---

## 4. TEKNOLOJİ YIĞINI (düzeltmeli)

| Katman | Crate | Amaç | v2 notu |
| --- | --- | --- | --- |
| Async runtime | `tokio` | Ajanlar, HTTP, zamanlayıcı | `rt-multi-thread`, worker sayısı config'ten |
| HTTP client | `reqwest` | llama-server + OpenRouter | timeout + retry interceptor'lı tek instance |
| JSON | `serde`, `serde_json` | Tüm şemalar | Tüm şemalar `deny_unknown_fields` yok; **forward-compat**: bilinmeyen alanlar korunur |
| CLI | `clap` (derive) | Komutlar/bayraklar | Exit code sözleşmesi §20.2 |
| Logging | `tracing` + `tracing-subscriber` (json) | Yapılandırılmış log | `RUST_LOG` override; log ≠ event (ikisi farklı akış) |
| Errors | `thiserror` (internal) + `anyhow` (boundary) | Katmanlı hata | Hata taksonomisi §16 |
| SQLite | `rusqlite` + `r2d2-sqlite` | State store, ledger, cache'ler | WAL modu, busy_timeout 5s, tek writer (§7.5) |
| GPU telemetri | `nvml-wrapper` (+ `nvidia-smi` fallback) | VRAM/kota | **Düzeltme:** `sysinfo` VRAM okuyamaz — §23 R6 |
| CPU/RAM | `sysinfo` | Sistem yükü | Local inference admission kontrolü (§10.1) |
| Rate limiting | `governor` | Cloud free tier (10 RPM, 5 concurrent) | Provider bazlı ayrı limiter |
| Async traits | `async-trait` | `Agent`, `Toolchain`, `InferenceProvider` | — |
| Config | `toml`, `dotenvy` | `ribozym.toml`, `agents.toml`, `models.toml`, `.env` | §18 |
| JSON Schema validasyonu | `jsonschema` | LLM çıktı şema kontrolü + `schema validate` komutu | Şemalar `schemas/` klasöründe (§11) |
| Test | `cargo test`, `mockall`, `wiremock`, `insta` (golden) | §21 | `cargo-deny`, `cargo-audit` CI'da |

---

## 5. AJAN KADROSU (A1–A12; doğrulama v2.45'te A11 verify şapkası oldu, arşivci v2.46'da A12 oldu)

### 5.1 Kadro

| Dept | Ajan | Unvan | Kapsam | Model tier | Girdi → Çıktı sözleşmesi |
| --- | --- | --- | --- | --- | --- |
| Governance | A1 | **Chief Architect & Referee** | Cloud (yerel yalnız uyarlama offload'u) | aşama bazlı — §5.4 tier haritası | `spec_form.md` + araştırma + context pack (§5.4) → `specs.json` + **`decisions[]` ADR (teknoloji dahil tüm proje kararları)**; `StandardDepartmentReport` + code → `ComplianceCheck` (§11.8) |
| Executive | A2 | CEO | Cloud | strong | `specs.json` → `tasks.json`; krizde replan |
| Reporting | A3 | Chief Reporter | Local | cheap | results + events → `StandardDepartmentReport.json` |
| Security & QA | A4 | Test Architect | Local (çoğunluk) / Cloud (zor benchmark yazımı) | cheap / strong | **test arşivi (`tests/`) canlı bakımı** — test/benchmark kurgulama, güncelleme, onarım (§5.9) |
| Engineering | A5 | Lead Coder | Local (boilerplate) / Cloud (algoritma) | cheap / strong | task + `context.md` → code artifacts + `ExecutionResult` (§5.10) |
| R&D | A6 | Researcher (R&D) | Local / Cloud | cheap→strong (hipoteze göre) | hipotez → A4 deney testi → A7 izole koşum → başarılı öneriler A2'ye (§5.11) |
| Security | A7 | Sandbox & Security | Local (izole ortam) | çoğunlukla deterministik | izole koşum + sürekli güvenlik taraması → `SecurityAudit` (§5.12) |
| Comms | A8 | Technical Writer & Repo Manager | Local | cheap | **proje dokümantasyonu + GitHub yönetimi**: README/API doc/CHANGELOG + commit/push/repo düzeni (§5.13) |
| Infra | A9 | Release & Distribution | Local (şablon) / Cloud (denge kuralı — §5.14 m2) | çoğunlukla deterministik + gerektiğinde cheap | projeye **dağıtım yüzeyi**: Dockerfile/CI/release workflow/installer + doğrulanmış kurulum komutları (§5.14) |
| Finance | A10 | Budget Planner / Resource Mgr | Local / OS | çoğunlukla deterministik (kural çıkmazında cheap) | bütçe yönetimi + zorluk analizi + aday eleme + benchmark tabanlı tier list (§5.15) |
| Web | A11 | Web Access & Verification | Cloud | strong (fetch) + **farklı model** (verify — exclusion'lu) | `WebQuery` → `WebFinding[]` + kör doğrulama → `VerificationScore` (§13); iki şapka, iki model |
| Archive | **A12** | Archive & Courier Agent (v2.30-31; v2.46'da A13'ten A12'ye yeniden numaralandı) | Local (OS/CLI) | çoğunlukla deterministik (gerektiğinde cheap API) | sistemden veri → zstd arşiv (§7.6); log kalitesi gözcüsü; iletişim zarfı kuryesi (§11.9); A3'ün tek veri kaynağı |

### 5.2 Ajan başına başarısızlık modu (her ajanın "çökerse ne olur" cevabı yazılıdır)

| Ajan | Tipik hata | Davranış |
| --- | --- | --- |
| A1 | Şemaya uymayan spec üretir | 2 deneme hakkı; yine olmazsa `SPEC_INVALID` hatasıyla kullanıcıdan düzeltme bilgisi istenir (eskalasyon 4) |
| A2 | Döngü içeren görev ağacı üretir | Zamanlayıcı döngüyü saptar, farkı A2'ye geri gönderir; 2 başarısızlıkta eskalasyon 2 |
| A5 | Derlenmeyen kod | Hızlı-hata döngüsü (§8): A7/A4'ün hata raporu kısaltılarak A5'e döner |
| A10 | Fiyat/benchmark API'si yanıt vermiyor | Son bilinen fiyat + %15 güvenlik payıyla çalışır (`PRICE_STALE` olayı yazar); bu da yoksa bulut çağrısı reddedilir (§8.2 m5). Tier list yerel veridedir — dış skor servisi çökse seçim bozulmaz |
| A7/A9 | Gerekli araç yok (cargo/docker) | `TOOLCHAIN_MISSING`; run başlamadan **ön-kontrol** (§12.4) yakalar |
| A11 | Zaman aşımı / boş sonuç | Farklı aramayla 2 deneme; sonuç gelmezse `WebFinding` boş döner, A6 bilgiyi "unverified" işaretler |
| A11 doğrulama | Arama ile aynı/aynı aileden model seçilirse | Seçici dışlama kuralı buna izin vermez (çağrı bazlı, model-ailesi bazlı; kanıt §10.3, R9) |

### 5.3 Ajan çalışma sözleşmesi (trait)

```rust
#[async_trait]
pub trait Agent: Send + Sync {
    fn id(&self) -> AgentId;                 // A1..A12
    fn consumes(&self) -> &[ArtifactKind];   // scheduler routing bilgisi
    fn produces(&self) -> &[ArtifactKind];
    async fn execute(&self, ctx: &mut AgentContext) -> Result<ExecutionResult>;
    // ctx: task, önceki artifact'ler, model_usage muhasebesi, tool gateway, event sink
}
```

Tamamen deterministik ajanlar (A7) LLM çağırmaz; `execute` yalnızca araç çalıştırır. Bu ayrım `consumes/produces` tablosunda `model: none` olarak işaretlenir. A9 ve A10 "çoğunlukla deterministik"tir: varsayılan yol 0 token'lık şablon/kuraldır; denge kuralları (§5.14 m2, §5.15) gerektirdiğinde seçiciye başvururlar.

### 5.4 A1 giriş: soru-cevap → analiz → düzeltme dönemi → kilit (v2.9; "tek ajan boş elle kalmaz" + düşük token ilkesi)

Giriş aşaması, sistemin kullanıcıyla en yakın temas kurduğu dört adımlı süreçtir:

**Aşama 1 — Soru-cevap (genel sorular + bağlama uyarlama; FSM durumu: Clarifying):** Kullanıcı tek cümleyle gelir ("takvim ekosistemi yapmak istiyorum"). Büyük analiz **henüz başlamaz**. Sorular `config/questions/core.md`'deki **tek genel setten** gelir: bir projeyi planlarken herkese sorulması gereken sorular (amaç, hedef kullanıcı, kapsam, kısıtlar, bütçe, teslim şekli, platform, riskler, başarı ölçütü...). **Projeye özel soru üretimi yoktur** (v2.11): ajanın işi, her genel soruyu **o anki konuya uyarlamak**tır — "hangi veriyi saklarsınız?" sorusu "takvim etkinlikleri ve kullanıcı bilgileri nerede saklanacak?" olur. Uyarlama **A1'in cheap moduyla** yapılır (ayrı ajan değil, §5.4A); çıktı yalnızca uyarlanmış soru cümlesidir (çok kısa = çok ucuz). Akış: her turda 2–4 soru sorulur → cevaplar `spec_form.md`'ye koddan yazılır → sonraki tur. Bitiş: genel setin tamamı yanıtlandığında ya da en fazla 3 tur dolduğunda. **Token ilkesi:** sorular dosyadan okunur (üretim yok), ajan yalnız uyarlar; cevapların analizi bu aşamada yapılmaz — büyük analiz çağrısı Aşama 2'de tek seferde yapılır.

**Aşama 2 — Analiz + açıklama (FSM: Researching):** Tam dolu `spec_form.md` **tek seferde** A1'e verilir (parça parça değil — tek büyük çağrı, çok sayıda küçük çağrıdan ucuzdur). A1 analiz eder — **araştırmasını kendisi yapar, A11'in getirdiği web verisiyle** (v2.36'dan beri A6 bu zincirde yok) — ve kullanıcıya **aynı kart formatıyla (§14.1 kural 7)** özet döner: *"fikrin şöyle: amaç X, kapsam Y, benzer projeler A/B, riskler Z."* Benzer-eser/rakip/risk araştırması bu aşamada A11 üzerinden tamamlanır.

**Aşama 3 — Düzeltme dönemi (FSM: Advising):** İki yönlü diyalog: A1 önerir ("şurayı şöyle yapalım mı?") → kullanıcı onaylarsa eklenir; kullanıcı önerir → A1 analiz eder, cevabını döner → kullanıcı karar verir. Döngü, kullanıcı "kilitli" deyene kadar sürer (danışma modeli, §5.5). Sistemin kullanıcıyla en yakın konuştuğu yer burasıdır.

**A1 = proje detaylarının sahibi (v2.6):** Diyalonun sonunda A1, **teknoloji seçimleri dahil** tüm proje kararlarını ADR biçiminde `decisions[]`'a yazar: dil, framework, veri katmanı, test araçları, toolchain. Her karar kaydında şu alanlar zorunludur: `choice` (seçilen), `alternatives_considered` (değerlendirilen seçenekler), `rationale` (gerekçe), `evidence` (A11 kanıtları). Rollere dikkat: **A1 karar verir, A5 seçilen çerçeveye göre kodlar** (`context.md`'deki "Technical Decisions" bölümü A1 kararlarının özetidir). A5 çerçeveyi beğenip değiştiremez — sapma yalnız `SpecRevised` ile A1'e karar talebi olarak gider.

Diyaloğun sonunda **kararlar, bütçe ve MVP kapsamı kilitlenir** (MVP = formdaki `must` maddeleri + kabul edilen öneriler) → spec imzalanır (`SpecValidated`) → A2 görev bölmeye geçer.

- **Kural:** A1'in emin olmadığı her şey ya soruya dönüşür ya da açıkça yazılmış bir varsayım olur. Sessiz tahmin yasaktır.
- **FinalReview'da varsayım denetimi:** teslimatta A1, varsayımların (`assumptions[]`) gerçekte doğru çıkıp çıkmadığını da kontrol eder; yanlış çıkan varsayım uyum yüzdesini (`spec_match_pct`) düşürür.
- **Kabul kriterleri çift damgalı:** taslağı A1 yazar; **A4, planlamadan önce her kriteri "test edilebilir mi?" süzgecinden geçirir** — ölçülemeyen kriter SpecRevised ile A2/A1'e geri döner (§6.4).

### 5.5 Kullanıcı danışma modeli — "kullanıcı belirler, AI analiz eder" (v2.8; öneri kanalının nihai biçimi)

Kullanıcı fikirlerini yalnız giriş aşamasında değil, **run boyunca** sorabilir: `ribozym suggest <spec_id> "acaba şöyle olsun mu?"`. Model nettir: **kullanıcı belirler, AI analiz eder ve cevabını verir; kullanıcı onaylarsa eklenir.** AI hiçbir zaman tek başına işe koymaz.

Süreç (her öneri için):

1. **Alım:** öneri `UserIdeaProposed` olayına yazılır — **asla görmezden gelinmez** (§24 kural 11).
2. **A1 etki analizi (paralel — run beklemeye girmez):** uyum (kilitli kararlarla çelişir mi), uygulanabilirlik (gerekiyorsa A11'den ek veri), **etkilenen kısımlar** (hangi görev/dosyalar; kuyrukta mı bitmiş mi — düzenleme sınıfları), **tahmini süre**, **tahmini maliyet** (A10), görev ağacına etkisi (A2: hangi görevler değişir/iptal olur).
3. **Tek kart cevabı → Evet/Hayır (v2.19; her öneri aynı formatta):**

   ```
   Öneri : "etkinliklere hatırlatma eklensin"
   Etki  : T2 veri modeli (tamamlanmış → yeniden koşar), T6 API (kuyrukta → genişler)
   Uyum  : D2 kararıyla uyumlu
   Süre  : +25 dk        Maliyet: +$0.06
   [Evet]  → görev sınırında entegre; tamamlanmışlara dokunuyorsa o görevler testleriyle regression
   [Hayır] → §5.4B ile BİREBİR AYNI döngü: açıklama alanı açılır ("neden olmadı, ne değişsin? serbest yaz")
             → A1 açıklamaya göre öneriyi/planı düzenler → yeni kart (etki/süre/maliyet güncellenir)
             → "Böyle uygun mu? Evet/Hayır" → döngü Evet'e veya kullanıcının vazgeçmesine kadar sürer;
               her açıklama öneri geçmişine kayda geçer
   ```

   - **Uygulanamaz** sonucunda kart yine gelir ama Evet seçeneği yoktur: kartta gerekçe ve **alternatif** öneri vardır. Seçenekler: [Vazgeç] ya da [Yine de isterim → etki, ağır yoldan tekrar hesaplanıp sunulur]. Kullanıcı "Yine de isterim"e açıklama yazabilir; A1 aynı döngüde yeniden değerlendirir.
   - **Tek etkileşim kalıbı (v2.20):** sistemde yalnızca bir diyalog düzeni vardır — **kart göster → Evet/Hayır → Hayırsa açıklama alanı açılır → düzelt → tekrar sor**. Karar kartları (§5.4B) ve öneri kartları (burası) bu tek kalıbı paylaşır; kullanıcı davranışı bir kez öğrenir, her yerde aynıdır.

### 5.4C A1 — TAM ŞARTNAME (KİLİTLİ, v2.21)

> Bu bölüm, v2.4–v2.20 iterasyonlarının **konsolide edilmiş nihai halidir**; A1'in bağlayıcı tanımıdır. Başka bir bölümle çelişirse bu kazanır. A1'in tasarımı kilitlidir — değişiklik ancak kullanıcının açık kararıyla ve yeni diff kaydıyla olur.

**Kimlik:** Baş Mimar ve Hakem. Tek ajan, beş farklı rol ("şapka"); kullandığı model aşamaya göre değişir (§5.4A). Karar yetkisi: teknoloji dahil tüm proje kararları. **İmza yetkisi yoktur** — her kilit ve onay kullanıcıdadır (ilke: AI danışmandır, kullanıcı karar vericidir).

### 5.6 A2 — CEO (tasarım v2.24; v2.25'te rapor-temelli denetim eklendi)

**Kimlik:** CEO — **A1'in teorisini pratiğe döker ve yürütmeyi yönetir.** A1'in çıktısı (kararlar + gereksinimler + MVP) henüz kâğıt üstü bir şartnamedir; A2 bunu gerçek iş parçalarına çevirir, ilgili ajanlara dağıtır, yürütmeyi taşır ve kriz anında planı yeniler. Başlangıç model önerisi strong; nihai seçimi A10 yapar (§10.3).

1. **Teoriyi en verimli şekilde göreve bölme:** A2, şartnameyi task'lara böler — **ne çok küçük** (her görev ayrı bir çağrı demektir = maliyet) **ne çok büyük** (tek seferde dev çıktı = kalite riski). Boyut ölçütü geçmiş veridir (`task_scores` tablosu, §7.2): hangi boyutlar hangi görev tipinde başarılı olduysa A2 o ölçekte böler.
2. **Dağıtım:** her görev, gereksinimin niteliğine göre doğru ajana atanır — planda gereksinim→görev haritası, ajan ataması ve dosya sahipliği yazılıdır.
3. **Plan kartı + tek etkileşim kalıbı:** Görev planı `gate:task_plan`'da kart olarak sunulur — görevler, gereksinim haritası, paralellik, **gerçek veriden hesaplanmış toplam süre/maliyet**. Onay akışı: **Evet/Hayır → Hayırsa açıklama alanı → A2 düzenler → yeni kart → tekrar** (§5.4B kalıbının aynısı). Uzun planlarda kart özet gösterir; tam liste "detay" ile açılır (liste zaten üretilmiştir, göstermek bedavadır).
4. **Gerçek veriden tahmin (v2.24):** A2 süre/maliyet **uydurmaz** — tahminler `task_scores` tablosundaki (§7.2) görev-tipi geçmişinden hesaplanır, A10 doğrular. Karttaki rakamlar LLM tahmini değil, geçmiş veridir; LLM yalnız görevlerin içeriğini ve sırasını önerir. Veri yoksa rakam uydurulmaz — kartta "tahmin yok" yazar (sessiz tahmin yasağının A2'deki karşılığı).
5. **İki katmanlı replan:** **Kapsam-içi replan** (gereksinimler aynı, görev dağılımı/sırası farklı — E2 krizlerinde) A2'nin **bağımsız** yetkisidir; kullanıcıya bilgi kartı gider, run durmaz. **Kapsam değişikliği** (yeni/çıkan gereksinim) yalnız suggest kanalıyla ve kullanıcı onayıyla olur (§5.5). **Maliyet eşiği (v2.28):** replanın tahmini ek maliyeti küçükse (varsayılan %10, config'te) A2 bağımsız uygular; büyükse etki kartıyla kullanıcı onayı ister.
6. **Rapor üzerinden sistem denetimi + kendi kendini düzeltme (v2.25-27):** A2'nin sistemi denetleme kanalı **raporlardır** — canlı müdahale yoktur. A3, tüm departmanların birleştirilmiş raporunu A2'ye gönderir; A2 raporu analiz eder, **aksayan kısımları saptar ve kullanıcıya değil, doğrudan sorumluya yönlendirir** — sistem kendi kendini düzeltsin.
   - **Aksama tanımı (v2.27):** yalnız gecikme/tekrar denemesi değildir — öncelikli ölçüt **yeterli verimde çıktı üretememe**dir: bir ajanın çıktısı kalite/verim olarak düşükse ve raporlarda öne çıkıyorsa (düşük test/doğrulama skoru, çok sayıda retry, gereksinimden sapma), o bir aksamadır ve sistem **verimi korumak için** düzeltmeye çalışır. Tespit başlangıçta koddan sabit eşiklerle yapılır (config'te: retry ≥ 2, verim skoru düşüşü, plandan %50+ sapma, bütçe %80) — zamanla ayarlanır.
   - **Yönlendirme:** görev hatası/düşük verim → ilgili ajanın düzeltme döngüsüne (E1; gerekirse E2 replan) · bütçe sapması → A10 · model kaynaklı başarısızlık → seçici (kara liste/model değişimi) · gecikme → zamanlayıcı.
   - **Değişiklik verimliliği (v2.28):** replan/düzeltme yine API ile yapılır ama **en verimli şekilde** — aynı sonucu **en az yeni görevle**: 4 yeni görev gerektirecek bir işi, mevcut görevlerle birleştirip daraltarak 2 görevle halletmek gibi. Gereksiz görev eklenmez; her yeni görev gerekçesiyle planda yer alır. Amaç: olabildiğince az çağrıyla verimi korumak.
   - **Sağlık kartı (v2.27):** küçük öz-düzeltmeler birikir, **run sonunda tek özet kart** olarak sunulur ("3 aksama oldu, hepsi çözüldü: ..."); önemli olaylar (E2 replan, bütçe %80, kapsam sorunu) **anında kart** üretir. Kullanıcıya, sistem çözebildiği sürece yalnız bilgi kartı gider; yetkiyi aşan durumlar (kapsam değişikliği, E4) suggest/NeedsUser ile gelir.
7. **Görev tanımı kuralı (v2.24):** her görev (a) tek ajana atanır, (b) tek dosya sahipliği taşır (çakışırsa zamanlayıcı sıraya dizer, §6.2), (c) tek oturumda bitmeye uygun büyüklüktedir, (d) en az bir gereksinime bağlıdır (§6.4). Ölçüt: görev çıktısı tek bir `ExecutionResult`'ta açıklanabilmelidir.

### 5.6A A2 — TAM ŞARTNAME (KİLİTLİ, v2.29) + YÖNETİM DEPARTMANI

> Bu bölüm, v2.24–v2.28 iterasyonlarının **konsolide edilmiş nihai halidir**; A2'nin bağlayıcı tanımıdır. Başka bir bölümle çelişirse bu kazanır. A2 kilitlidir — değişiklik yalnız kullanıcının açık kararıyla ve yeni diff kaydıyla olur.

**A1 + A2 = YÖNETİM DEPARTMANI.** A1 (Baş Mimar ve Hakem) projeyi **tanımlar**: kararlar, gereksinimler, kabul kriterleri, bütçe — kâğıt üstü şartname. A2 (CEO) bunu **pratiğe döker ve yönetir**: görevler, dağıtım, yürütme, denetim — gerçek iş. İkisi birlikte sistemin beyin ve yönetim katmanıdır; kalan 10 ajan icra/destek birimleridir.

**Görev zinciri:**

| # | Görev | Kesin kural |
| --- | --- | --- |
| 1 | Teori → pratik: en verimli task bölme | Ne çok küçük (çağrı maliyeti) ne çok büyük (kalite riski); boyut ölçütü `task_scores` tablosu geçmişi (§7.2) |
| 2 | Departmanlara dağıtım | R→G haritası + ajan ataması + dosya sahipliği planda yazar |
| 3 | Plan kartı + onay | Tek etkileşim kalıbı (§5.4B): Evet/Hayır → Hayırsa açıklama alanı → düzelt → tekrar; uzun planda özet kart + istenince detay (depo-önce) |
| 4 | Deterministik tahmin | Süre/maliyet `task_scores` tablosu (§7.2) + A10 doğrulaması; **LLM tahmini yazmaz**; veri yoksa "tahmin yok" |
| 5 | Replan iki katman | Kapsam-içi: otonom (bilgi kartı, run durmaz); ek maliyet ≤ %10 (config) otonom, üstü etki kartı + onay. Kapsam-değişikliği: yalnız suggest + kullanıcı onayı |
| 6 | Rapor-temelli denetim + öz-düzeltme | A3'ün birleştirilmiş raporlarını analiz eder; **aksama = yeterli verimde çıktı verememe** (verim düşüşü raporlarda öne çıkar) → **sorumluya yönlendirir** (fix döngüsü / A10 / selector / scheduler), kullanıcıya değil. Küçük düzeltmeler → run sonunda sağlık kartı; önemli olaylar (E2, bütçe %80, kapsam) → anında kart |
| 7 | Değişiklik verimliliği | Replan/düzeltme yine API ile ama **en az yeni görevle** — 4 yerine 2 görev gibi; gereksiz görev yok, her yeni görev gerekçeli |
| 8 | Granüllük | Tek ajan + tek dosya + tek oturum + ≥1 gereksinim |

**Kesin kısıtları:**
1. **Kapsamı değiştiremez** — önerir, kullanıcı onaylar (suggest kanalı).
2. **Tahmin uydurmaz** — rakamlar veriden gelir, asla LLM'den değil.
3. **Aksayan işi kullanıcıya havale etmez** — sorumluya yönlendirir; kullanıcıya yalnız bilgi/önem kartı gider.
4. **Plan onaysız başlamaz** — görev planı kapısı (gate:task_plan) kullanıcıya aittir.
5. **Çıktı ekonomisi + asimetrik iletişim + depo-önce + arşivden bağlam** (§14.1, §7.6) tüm çağrılarında geçerlidir.

### 5.7 A3 — Baş Raportör (KİLİTLİ, v2.30-32)

**Kimlik:** Baş Raportör — sistemin **tüm raporlama işlerinin tek sahibi**. Kendisi hiçbir iş üretmez; 12 departmanın ürettiğini toplayıp anlamlı hâle getirir. Hem A2'nin denetim sistemini (§5.6A) hem teslimattaki nihai raporu o üretir. Başlangıç model önerisi cheap — çünkü sayısal bölüm zaten koddan gelir, LLM yalnız anlatı için çalışır.

1. **Veri kaynağı arşivdir (A12 üzerinden):** Her departmanın çıktılarını **A12 Arşiv Ajanı** çeker ve §7.6'daki arşive sıkıştırılmış yazar (`runs/<spec_id>/archive/`). A3 rapor üretirken bu arşivi koddan açar, düzenler ve anlamlı hâle getirir — LLM'e ham log değil, yapılandırılmış veri gider.
2. **Rapor = koddan gelen iskelet + veri analitiği (v2.31):** Sayısal bölüm (süre, maliyet kırılımı, tekrar denemeleri, uyum yüzdesi, gereksinim izlenebilirlik haritası) **koddan, SQLite/arşivden** doldurulur. LLM'in rolü **yorum yapmak değil, veri analitiği yapmaktır**: bir veri analisti gibi sayılardan anlamlı sonuç çıkarır — eğilimler, darboğazlar, neden-sonuç sinyalleri. **A2'ye giden raporlara yorum katmaz** — yalnız çıkarılmış sonuçları yazar; **kullanıcıya giden raporda** ise açıklayıcı, okunur bir sunum yapar (§14.1).
3. **Ara rapor ritmi:** Her görev sınırında koddan kısa durum özeti (0 token); her aşama/tur bitiminde LLM'li kısa analiz; A2 istediği an tam rapor ister. A2'nin rapor-temelli denetimi (§5.6A) bu akışla beslenir.
4. **Nihai raporun mührü:** `StandardDepartmentReport.json` teslimat kapısından önce üretilir, A1 onayıyla **dondurulur** — sonradan değiştirilemez (güven zinciri).
5. **Tek raporlama yetkisi:** Kullanıcıya ve A2'ye giden hiçbir rapor A3 dışından üretilmez; diğer ajanlar yalnız ham çıktı + olay üretir. (Log ≠ rapor: log otomatik kayıttır, rapor A3'ün analitik üretimidir.)
6. **Rapor türlerinin sınırları (v2.32):** A3'ün ürettiği üç tür: **(a) nihai rapor** (mühürlü StandardDepartmentReport) · **(b) ara raporlar** (A2'ye: görev sınırı özetleri + aşama analizleri) · **(c) sağlık kartı içeriği** (run sonu aksama özeti; kartı A2 sunar, içeriği A3'tür). Rapor **olmayanlar**: canlı sorgu ekranları (`status`, `budget show` — kodun doğrudan gösterimidir), kapı kartları (A1/A2'nin karar dosyaları), ham dosyalar (test sonuçları vb. — A3 bunları rapora *dahil eder*, kendisi üretmez).

### 5.8 A12 — Arşiv & Kurye Ajanı (KİLİTLİ, v2.31-32)

**Kimlik:** Sistemin veri hareketinin ve **log kalitesinin** sahibi — log ne kadar iyiyse sistem o kadar iyi çalışır; A12 bu özü korur. Kadroya "çoğunlukla deterministik" olarak girdi.

1. **Tüm sistemden arşiv:** her köşeden veri toplar (ajan çıktıları, olaylar, ara ürünler) → zstd ile sıkıştırılmış arşiv bölümleri yazar (§7.6).
2. **Log kalitesi gözcüsü:** loglar koddan üretilir ama beklenmeyen/exstra veriler de olabilir; A12 log standardını korur — tam, anlamlı, geri okunabilir loglar. Zayıf log bölümü görürse işaretler ya da düzeltilmesini ister.
3. **Kurye:** ajanlar arası iletişim zarflarını oluşturur/damgalar (seq, correlation_id) ve taşır; her zarfın kopyası arşive düşer (§11.9).
4. **Denge kuralı (v2.31):** A12 **yalnızca koddan iyi yapabildiklerini kendisi yapar** — toplama, sıkıştırma, şema denetimi, damgalama. Koddan yapılamayan işlerde (anlam çıkarma, eksik/bozuk veriyi tamamlama, kalite değerlendirme) **API'ye başvurur** — gereksiz LLM çağrısı yok, gerektiğinde de kaçınmaz.

### 5.9 A4 — Test Mimarı (Test Kurgulayıcı, tasarım v2.33-34)

**Kimlik:** Sistemin **test arşivinin sahibi ve bakıcısı** — test kurgulayıcı. Her projedeki `tests/` klasörünün kalitesi bu ajana bağlıdır; klasör durağan bir dosya yığını değil, **koduyla birlikte yaşayan bir koleksiyondur**.

1. **Test arşivi mülkiyeti:** `tests/` klasörü **yalnız A4'ün mülkiyetindedir** — A5 (kod yazarı) test dosyası yazamaz (kendi testini kendi geçme kalkanı; R3 riski). Testler kabul kriterlerinden **bağımsız** türetilir: önce kabul kriteri, sonra onu ölçen test.
2. **Canlı arşiv bakımı:** testler **güncel tutulur** — kod değişince etkilenen testler güncellenir, bozuk/eski testler onarılır ya da emekliye ayrılır, gereksinim başına **yeni test üretilir**. Arşiv her run'da birikir: kanıtlanmış test kalıpları bir **test-şablon kütüphanesine** dönüşür.
3. **Benchmark üretimi:** projenin niteliğine uygun büyük benchmarklar — sınır durumlar, stres, performans.
4. **Yazım/koşum ayrımı (v2.34):** test **yazdırma** işleminde model zorluğa göre seçilir: basit birim testlerini **yerel model** yazar (bedava), büyük/stres benchmarklarını **güçlü bulut modeli** yazar. Testlerin **ÇALIŞTIRILMASI** ise her zaman yerel makinede yapılır (`cargo test` vb.) — koşum için LLM gerekmez.
5. **Hakkaniyet garantisi:** testler sistemi **gerçekten ölçen** adil testler olmalıdır — "geçecek şekilde yazılmış" test yasaktır. Güvenceler: (a) her test bir kabul kriterine bağlanır (izlenebilirlik), (b) test değişiklikleri kod değişikliğinden **bağımsız kaydedilir** (testi koda uydurma girişimi görünür olur), (c) test kalitesi A2'nin rapor denetimine tabidir.
6. **Koşum verimliliği (v2.33):** düzeltme turunda yalnız **etkilenen alanın testleri** çalışır (etkilenen dosyalar koddan belirlenir); görev bitiminde **tam paket** koşar. **Kararsız test yönetimi:** 2 koşuda farklı sonuç veren test `flaky` işaretlenir (hata sayılmaz, kayda geçer, A4'ün onarım listesine düşer). **Kapsam eşiği:** kritik yollar %100, genel %70+ (config) — her kabul kriteri en az bir teste bağlıdır.

> **Kapsam notu (v2.34):** A4'ün görevi test **kurgulamaktır** — oluşturma, güncelleme, onarım. Test koşumunda çıkan kod hatalarını yakalamak ana akışın işidir (§6.3: A4 koşar → hata → A5 düzeltir); ayrı bir "hata listesi" üretimi A4'ün görevi değildir.

### 5.10 A5 — Baş Kodlayıcı (KİLİTLİ, v2.35)

**Kimlik:** Baş Kodlayıcı — **projeyi kodlayan ajan.** Tek işi budur: kendisine atanan görevin kodunu, A1'in kararları çerçevesinde yazmak. Ekstra mekanizma yok — A5'i disiplinli tutan şey kendi kuralları değil, sistemin ona çizdiği sınırlardır:

1. **ADR çerçevesi:** A1'in kilitlediği kararlarla kodlar (dil, framework, veri katmanı); framework beğenmezse sessizce değiştiremez — `SpecRevised` ile A1'e karar talebi götürür (§5.4C).
1a. **Girdisi `context.md` (v2.46 netleşti):** A1'in Researching fazının çıktısıdır — spec özeti + `decisions[]` + gereksinimler + A11 kanıtlarının derlenmiş hâli (§5.4). A5 bağlamı konuşmadan değil, bu dosyadan alır (arşiv ilkesi, §7.6).
2. **Dosya sahipliği:** yalnız görevine ait dosyaları yazar; **`tests/` klasörü ona yasaktır** (testleri A4 kurgular, §5.9).
3. **Düzeltme döngüsü:** A4/A7'nin yakaladığı hataları düzeltir — en fazla 3 tur, sonra eskalasyon (§8.1).
4. **Model seçimi:** basit/tekrar işler yerel model adayı, karmaşık algoritmalar güçlü bulut modeli — seçimi her çağrıda seçici yapar (§10.3).
5. **Genel kurallar:** arşivden bağlam (§7.6), zarfla iletişim (§11.9), çıktı ekonomisi (§14.1) — tüm ajanları bağlayan kurallar.

### 5.11 A6 — Araştırmacı (R&D Laboratuvarı, KİLİTLİ, v2.36)

**Kimlik:** Gerçek bir Ar-Ge laboratuvarı — **A1'in araştırmasını yapmaz** (A1, A11 verisiyle kendi araştırmasını kendisi yapar). A6'nın işi **bilimsel yöntemle R&D**: iyileştirme hipotezi üretir, dener, kanıtlayabildiğini sisteme önerir. Run süresince güncel teknolojileri takip eder.

1. **Hipotez üretimi:** güncel teknolojileri/yaklaşımları izler (A11 ile), mevcut projeye uygulanabilecek **iyileştirme hipotezleri** üretir ("X kütüphanesi G3'ü 2 kat hızlandırabilir" gibi).
2. **Deney zinciri (bilimsel yöntem):** hipotez → **A4'ten deney testi kurgulaması** (hipotezi ölçen test) → **A7'de izole ortamda koşum** (ana sistemi hiç etkilemeden) → ölçülebilir sonuç.
3. **Kanıtlanan hipotezler → A2:** işe yaradığı kanıtlanan hipotezler "eklenebilecek öneriler" olarak A2'ye gider → **suggest kanalı (§5.5)**: A2 etki analizi yapar (maliyet/süre/uyum), kullanıcıya kart olarak sunulur, **Evet/Hayır**. Yani R&D'nin sisteme dokunması yalnız kullanıcı onayıyla olur.
4. **Bütçe:** R&D harcamaları araştırma kotasından beslenir (`research_quota_pct`, config; §5.15 A10 bütçe çerçevesi); A10 kelepçesi geçerlidir; her deneyin maliyet üst sınırı vardır.
5. **Öncelik + yaşam süresi:** run sırasında **düşük öncelikli arka plan** işi olarak çalışır — ana akışı asla yavaşlatmaz; zamanlayıcının boş kapasitesinde çalışır. **Run dışında çalışmaz** (§1.2 kapsam dışı: kalıcı daemon yok) — "sürekli R&D" run'ın yaşam süresi içindedir.
6. **R&D hafızası (v2.46):** her hipotez + deney + sonuç **A12 arşivine** yazılır (§7.6); sonraki run'da A6 arşivden önceki denemeleri okur — aynı hipotez iki kez denenmez, yarıda kalan deneye devam edilebilir. Run'lar arası sürekliliğin tek kaynağı arşivdir, ajanın kendi belleği yoktur (§7.6'daki "her sabah yeni işçi" ilkesi A6 için de geçerlidir).

### 5.12 A7 — İzolasyon & Güvenlik (KİLİTLİ, v2.36)

**Kimlik:** **Her türlü izole ortam işinin sahibi + antivirüs gibi sürekli tarayan güvenlik ajanı.** Ağırlıklı olarak koddan (deterministik araçlarla) çalışır; zorlu güvenlik analizlerinde seçiciden model alır.

1. **İzole ortam (tek kapı):** tüm riskli çalıştırma işleri A7'nin yönettiği izole konteynerlerde olur — proje testleri, **A6'nın deneyleri**, doğrulanmamış kod. Ana sisteme dokunan hiçbir şey izolesiz çalışmaz (§12 izolasyon politikasıyla birlikte).
2. **Test/deney koşumu:** A4'ün kurguladığı testleri ve A6'nın deney testlerini izole ortamda çalıştırır, sonuçları ölçülebilir biçimde döner.
3. **Sürekli güvenlik taraması (antivirüs modu):** her dosya girişinde ve her komutta — statik analiz, `cargo audit`, gizli anahtar taraması, izin listesi denetimi, şüpheli davranış izleme. Sürekli ve otomatiktir; tek seferlik bir "güvenlik aşaması" değildir.
4. **Bulgu akışı:** güvenlik açığı → doğrudan A5'e düzeltme talebi (E1); kritik bulgu (`SEC_SECRET_FOUND` gibi) → run durur, raporlanır (§12.5).
5. **Model:** ağırlıklı olarak deterministik araçlar; derin açık analizi gibi işlerde seçiciden uygun model alır (denge kuralı, §5.8 m4 ile aynı ilke).

### 5.13 A8 — Teknik Yazar & Repo Yöneticisi (KİLİTLİ, v2.37-38)

**Kimlik:** Teknik Yazar + Repo Yöneticisi. İki görevi var: **(1) projenin dokümantasyonunu yapmak, (2) GitHub tarafını yönetmek** — commit'leri atmak, repo düzenini korumak.

**Dokümantasyon:**
1. **Dil ve uzunluk politikası (kullanıcı kararı):** dokümantasyon **İngilizce** ve **kısa ve öz** — ne o kadar kısa ki bilgi eksik kalsın, ne o kadar uzun ki laf dolgusu olsun.
2. **İçerik gerçeklikten gelir:** doküman A1'in kararlarını, gerçek dosya yapısını ve gerçek komutları yansıtır — var olmayan özellik/komut yazamaz; README'deki kurulum komutları A7'nin izole ortamında bir kez çalıştırılarak doğrulanır.
3. **Çıktı sabit üçlü:** README + API dokümanı + CHANGELOG; fazlası ancak A1 kararıyla.
4. **Ekonomi:** özet ilk 10 satırda (depo-önce); öz değişmedikçe kod değişimlerinde doküman yeniden yazılmaz, yalnız etkilenen bölüm güncellenir (§14.1).
5. **Üretilen her şey projenin dosyasıdır, rapor değil:** rapor dili yoktur (o A3'te).

**GitHub yönetimi (v2.38):**
6. **Commit disiplini:** Conventional Commits (`feat`, `fix`, `test`, `docs`, `chore`...); her başarılı ajan çıktısı ayrı commit; **başarısız çıktı asla commit edilmez**. Commit mesajları şablonla (koddan) üretilir — LLM'e gerek yoktur.
7. **Push politikası (güvenlik kapısı):** push **yalnız teslimat onayından (gate:delivery) sonra**; her zaman `ribozym/<spec_id>` dalına; **`main`'e asla**. A1 onayı olmadan dış dünyaya hiçbir şey gitmez.
8. **Repo düzeni:** repo yapısı (klasörler, `.gitignore`, LICENSE başlığı) A1 kararlarına ve şablona uygun kurulur ve bozulmaz; dal listesi temiz tutulur (tamamlanan spec dalları teslimden sonra arşivlenir).
9. **Sınır:** A9'un ürettiği dağıtım dosyalarını (Dockerfile, CI, release workflow, installer) repo'ya A8 işler; üretim A9'da kalır — **A8 yayımlar, üretmez**.

### 5.14 A9 — Sürüm & Dağıtım (KİLİTLİ, v2.40)

**Kimlik:** Projenin **son kullanıcı tarafından kurulabilir-çalıştırılabilir olmasının sorumlusu** — teknik dağıtım yüzeyini üretir. Tek cümlelik amaç: *"GitHub'da repoyu gören biri kendi makinesine indirir, kurar, çalıştırır"* — bunu sağlayan her şey A9'ün çıktısıdır. İzole ortam işi **A7'dedir**, karışmaz: A7 = ribozym'in kendi güvenlik izolesi, A9 = üretilen projenin kullanıcıya dönük dağıtım yüzeyi.

1. **Üretim çıktıları (dağıtım yüzeyi):** Dockerfile (+ gerekiyorsa docker-compose), CI şablonu (`.github/workflows/ci.yml`), **GitHub Release workflow** (cargo-dist benzeri şablon — işletim sistemi bazlı derleyip Release'e koyar), **kurulum betiği** (install.sh / install.ps1 + checksum), gerekli derleme dosyaları ve **doğrulanmış kurulum komutları** — kurulum yolunu A9 uçtan uca test eder, doğrulanmış komutları A8'e verir (§5.13 m2'deki "README'de doğrulanmış kurulum komutları"nın üreticisi önceden tanımsızdı; bu boşluk burada kapandı). Tümü A1'in kilitlediği teknoloji kararlarına göre (toolchain, sürümler).
2. **Determinizm–API denge kuralı (v2.40; A12 §5.8 m4 ile aynı ilke):** varsayılan yol koddur — şablon + kararların yerleştirilmesi, 0 token. Şablonun yetmediği iş (şablonda olmayan teknoloji yığını, standart-dışı derleme bağımlılığı) için seçiciden cheap-tier API çağrısı yapılır (§10.3; A10 bütçe kelepçesi her zaman geçerli). **Gereksiz API çağrısı yok; gerektiğinde de kaçınmaz.** API çıktısı her durumda şema kontrolünden geçer (§3.1).
3. **Doğrulama:** ürettiği her dosya (docker build, release workflow, kurulum betiği, kurulum komutları) A7'nin izolasyonunda **bir kez çalıştırılarak** doğrulanır; geçmeyen dosya üretilmiş sayılmaz.
4. **Yayımlama yetkisi yok:** ürettiği dosyaları repo'ya A8 işler (commit/push A8'de, §5.13 m6-7); A9 dış dünyaya asla dokunmaz. Release workflow dosyasının üretimi A9'da, tetiklenmesi (tag/push) A8'dedir.
5. **Kapsam dışı:** Kubernetes/üretim-dağıtım yok (§1.2); **paket kayıtlarına yayınlama (crates.io, Homebrew, winget) kapsam dışı** — A9 repo-düzeyi dağıtım üretir (kayıt yayını v1.x adayı).

### 5.15 A10 — Bütçe Planlayıcı & Model Tedarikçisi (KİLİTLİ, v2.44)

**Kimlik:** CFO ve alım uzmanı — **pasif harç-kelepçesi değil, aktif bütçe yöneticisi.** Ağırlıklı olarak koddan çalışır: bütçe çerçevesini yönetir, görev zorluğunu analiz eder, havuzdan model seçer, **kendi benchmarklarını koşturup kendi model sıralamasını (tier list) üretir**. Üçüncü taraf skor servislerine (Artificial Analysis vb.) birincil itibar etmez — yalnız kendi ölçtüğüne güvenir.

**Görev zinciri:**

| # | Görev | Kesin kural |
| --- | --- | --- |
| 1 | Bütçe yönetimi | **Bütçeyi kullanıcı dışarıdan belirler:** aylık toplam (varsayılan $5.00) + test kotası (varsayılan %10) config'te kullanıcı tarafından yazılır. A10 tahsis yapmaz — verilen çerçevede harcamayı yönetir, kontrol noktalarını işletir (§8.2), bütçe tükenmeye yakın uyarır; çerçeveyi ancak kullanıcı değiştirir. Run bütçesi (spec'te, A1 yazar) iş bütçesinden kesilir. |
| 2 | Görev zorluğu analizi | **Determinizm–API denge kuralının ana örneği (v2.44):** birinci yol kural setidir — görev tipi, dosya sayısı/boyutu, bağımlılık derecesi, `task_scores` geçmişi → çoğu görev bununla derecelenir (0 token). Kural net karar veremezse (görev metni "basit düzeltme" mi "karmaşık algoritma" mı belirsizse) **tek cheap-tier LLM sınıflandırma çağrısı** — girdi görev özeti, çıktı yalnız zorluk etiketi + gerekçe (kısa, yapılandırılmış). Sonuç `tasks` kaydına zorluk alanı olarak yazılır; hangi yolla derecelendiği (`rules`/`llm`) kayıtta kalır. |
| 3 | Aday model eleme | OpenRouter API (model listesi + canlı fiyat) ve Artificial Analysis API (kalite skorları) ile havuz taranır. **Aranan kriter: ucuz VE performanslı.** Elemeden geçen adaylar test kuyruğuna girer; ölçülmemiş model havuza giremez. |
| 4 | Benchmark üretimi (A4'e iş) | A10, **görev-tipi benchmarklarını** A4'e yazdırır — gerçek iş yükünü ölçen testler (§5.9'daki test kurgulama yeteneğinin model testine kullanımı). Benchmark üretimi, test kotasından ayrı bir plan kalemidir. |
| 5 | İzole model testi | Aday modeller, test kotasından ödenen API çağrılarıyla **A7'nin izolasyonunda** (§5.12) benchmarklardan çalıştırılır; ölçülen şey gerçek maliyet + gerçek kalitedir. Test trafiği gerçek run'lardan tamamen ayrıdır. |
| 6 | Tier list (görev bazlı) | Çıktı: `görev tipi → sıralı model listesi (benchmark skoru + fiyat + gerçek başarı)` — `task_scores` tablosunda yaşar (§7.2). Model seçimi **görevin zorluk derecesine göre bu listeden** yapılır (§10.3); gerçek run verisi listeyi sürekli günceller. |
| 7 | Kelepçe (korunur) | §8.2 kontrol noktaları aynen geçerli: çağrı öncesi tahmin + sonrası fiili; %80'de uyarı + fiyat-lehine kayma + test harcaması durur; %100'de bulut çağrısı reddedilir. E3 yükseltme onayı A10'dadır (§8.1). |

**Determinizm–API denge kuralı (v2.44; A12 §5.8 m4, A9 §5.14 m2 ile aynı ilke):** her iş, **o işi daha iyi yapanla** yapılır. Aritmetik, sıralama, eşik, kelepçe = koddan (bu işlerde LLM'den hem hızlı hem daha tutarlı). Anlam gerektiren tek iş = zorluk analizinin kural çıkmazı (tek cheap LLM çağrısı). LLM çağrısı şemaya tabidir; A10 bütçe kelepçesi kendisine de geçerlidir (kendi sınıflandırma çağrısı da model_usage'a yazılır).

**Bootstrap / kalibrasyon (süre sınırlı):** havuz boş başlamaz — kullanıcının **13 gün boyunca 6 güçlü API'ye sınırsız erişimi** vardır (bitiş ≈ 2026-09-25). `ribozym calibrate`: A4 benchmark setini üretir → 6 API'nin tamamı A7 izolinde, sınırsız erişimle (fiili maliyet 0, test kotası yanmaz) çalıştırılır → ilk tier list yazılır. **Takvim (§22):** planlama 2026-09-12'de bitti, kodlama en fazla 3 gün sürer — kalibrasyon kodlamanın 1. gününün sonunda koşar, pencere bolca yeterlidir. Pencere kapandıktan sonra yeni model eklemek normal yoldan yapılır: test kotasından ölçülür, pencere gerekmez.

**Kesin kısıtları:**

1. **Ağırlıklı olarak modelsizdir** — kararlar veriden alınır (metadata + geçmiş + dış API rakamları); LLM yalnız zorluk-analizi kural çıkmazında, tek cheap sınıflandırma çağrısı olarak çalışır (m2, denge kuralı). Tahmin uydurmaz, ölçmediği modele puan vermez.
2. **Bütçeyi kullanıcı belirler** — A10 yalnız yönetir; test kotası dışına benchmark için para harcayamaz, çerçeveyi kendisi genişletemez.
3. **Test trafiği izoledir** — benchmark koşumları A7'nin izolasyonundadır (§5.12); gerçek run'a sızmaz.
4. **Model seçimi tek yetkedir** — ajanlar model isteyemez, zorluk da A2'den gelmez; görev analizi + seçim dahil tüm model kararları A10'dadır (§10.3), A11 arama ≠ doğrulama modeli kuralı dahil.
5. **Şeffaflık** — her tahsis, sıralama değişimi ve kara liste olaya + A3 raporuna ham veri olarak yazılır.

**Görev zinciri (yaşam döngüsü sırasıyla — A1):**

| # | Rol | Faz (FSM) | Girdi → Çıktı | Kesin kural |
| --- | --- | --- | --- | --- |
| 1 | Mülakatçı | Clarifying | tek cümle → dolu `spec_form.md` | Genel soru seti + her sorunun bağlama uyarlanması (**cheap mod**, ayrı ajan değil); tur başına 2–4 soru, en fazla 3 tur; **cevap analizi yok**; cevaplar koddan forma yazılır |
| 2 | Analist | Researching | dolu form → analiz kartı | Tek büyük çağrı (**strong**); **araştırmayı A11 verisiyle kendisi yapar (v2.36'dan beri A6 bu zincirde yok)**; tek format ailesi (§14.1 k7) |
| 3 | Danışman | Advising | diyalog → kabul/ret kayıtlı öneriler | İki yönlü diyalog; kullanıcı **"kilitli"** deyene kadar; her açıklama geçmişe yazılır |
| 4 | Karar verici | Advising → SpecValidated | `specs.json` + `decisions[]` ADR | Her karar: choice + alternatives_considered + rationale + evidence **zorunlu**; bütçe + MVP burada kilitlenir; A5 sapması yalnız `SpecRevised` ile geri gelir |
| 5 | Kart ekranı | gate:spec | kartlar → onay | Tek etkileşim kalıbı (§5.4B): Evet/Hayır; **Hayır → açıklama alanı (LLM'siz açılır) → kullanıcının açıklamasına göre düzenleme turu (az çıktı) → tekrar Evet/Hayır**; kartla ilgili hazır sorular depodan cevaplanır |
| 6 | Fikir analisti | run boyunca | suggest → etki kartı → Evet/Hayır döngüsü | Tek kart: etkilenen kısımlar + tahmini süre + tahmini maliyet (§5.5); analiz **paralel** yürür, run durmaz; entegrasyon görev sınırında; bitmiş işe dokunma = **ağır yol**; "uygulanamaz" kartında Evet yoktur |
| 7 | Hakem | FinalReview | rapor + kod → `ComplianceCheck` | Üçlü denetim: gereksinim %100 + varsayım denetimi + karar uyumu; sonuncusu **koddan kanıtla** (A7/A4: bağımlılık manifestosu, import taraması, test envanteri); çift imzanın ayağı |

**Kesin kısıtları:**

1. **Sessiz tahmin yok** — emin olmadığı her şey ya sorudur ya açıkça yazılmış `assumptions[]`; FinalReview varsayımları denetler.
2. **Onaysız entegrasyon/kilit yok** — en ucuz, en mantıklı bulduğu fikri bile tek başına eklemez.
3. **Kabul kriterleri A4 damgası olmadan planlanamaz** (§6.4).
4. **İletişim disiplini:** tek format ailesi + asimetrik iletişim + depo-önce + çıktı ekonomisi (§14.1, k1–k8).
5. **Model bilinci:** tier haritasına uyar (§5.4A) — uyarlama cheap (yerel model adayı), analiz/karar/denetim strong; kendi önerilerinin üretimi cheap.
6. **Bağlam = arşiv, konuşma değil (v2.22):** her aşama çağrısının girdisi §7.6 arşiv katmanından gelen ilgili bölümdür — aşamalar arası tutarlılık hatıra ile değil, arşiv tutanaklarıyla sağlanır; kanıt alanı arşivle eşleştirilir.

- Tüm öneri döngüsü `UserIdea*` olaylarıyla izlenir; nihai raporda öneri denetimi bölümü yer alır (ne önerildi, etki neydi, karar ne oldu).

- **Onaysız entegrasyon yok:** AI'nın "ucuz ve mantıklı" bulduğu fikir bile kullanıcı onayı olmadan eklenmez — AI danışmandır, karar verici değildir.
- **Zamanlama kuralı:** onaylanan öneriler yalnız **görev sınırında** uygulanır — koşmakta olan görev asla ortasında değişmez, yarım iş bozulmaz.
- **Run bozulmaz garantisi (v2.16):** öneri analizi mevcut planla **paralel** yürür — run analiz/onay için beklemez, çalışmaya devam eder. Onay bekleyen öneri run'ı durdurmaz; kullanıcı döndüğünde cevap kart olarak sunulur. Entegrasyon görev sınırında uygulanır; iptal edilen kuyruk görevleri silinmez, `skipped` olarak işaretlenir. Tüm adımlar ledger + olay kaydına yazıldığı için çökme/resume senaryosunda da tutarlıdır.
- **Düzenleme sınıfları (v2.17):** önerinin dokunduğu yere göre üç yol:
  - **Sınıf 1 — kuyruktaki işe dokunur:** normal yol; hızlı analiz, görev sınırında entegrasyon (ana akış).
  - **Sınıf 2 — tamamlanmış işe dokunur:** **ağır yol — asla hızlı kabul edilmez.** Etkilenen dosya/görev listesi koddan çıkarılır, yeniden-iş maliyeti raporlanır ("3 tamamlanmış görevi etkiler: +N dk, +$X"). Kullanıcı açık onay verirse etkilenen görevler **yeniden çalıştırılır** (mevcut testler tekrar koşar). Ayrıca erteleme seçeneği sunulur: "bunu şimdi değil, mevcut işler bitince yapalım".
  - **Sınıf 3 — kilitli karara/teslim edilmişe dokunur:** yeni ADR + tam replan gerekir; varsayılan davranış: run bozulmadan not edilir, run sonrası değerlendirilir.
- **Kilitli karar çelişkisi:** öneri kilitli kararlarla çelişiyorsa entegrasyon ancak A1'in **yeni ADR kararı** + kullanıcının açık onayıyla olur.

### 5.4A A1 model-tier haritası (v2.12; v2.23'te seçiciye bağlandı)

A1 tek ajan, beş farklı rol — ama **tek model değildir**: her aşama farklı bir çağrı tipidir. Aşağıdaki tier'lar **başlangıç önerisidir; nihai seçim A10'a aittir (§10.3)**: her çağrı bir `task_type` + zorluk etiketiyle gelir, A10 `task_scores` tablosundaki (§7.2) görev-tipi geçmişine ve tier list'ine göre bütçe içinde en uygun modeli seçer (aşama-model kilidi yoktur).

| Aşama | İş | Başlangıç tier önerisi |
| --- | --- | --- |
| Clarifying | Soru bağlama uyarlama | cheap (yerel model adayı) |
| Researching | Tam form analizi + açıklama | strong (tek büyük çağrı) |
| Advising | Öneri analizi (kullanıcı fikri) | strong |
| Advising | A1'in kendi önerilerinin üretimi | cheap |
| Karar kilidi | ADR yazımı + bütçe/MVP kilidi | strong |
| FinalReview | ComplianceCheck | strong + **koddan kanıt** (öz-denetim güvencesi, §17) |

Soru-uyarlama yeteneği **ayrı bir ajan değildir** — A1'in cheap modudur; tek ajan kimliği ve tek `decisions[]`/`ComplianceCheck` sorumluluğu korunur.

### 5.4B Karar kartları — token-verimli teknoloji onayı (v2.13)

Teknoloji kararları kullanıcıya **serbest sohbetle değil, kart ekranıyla** açıklanır ve onaylanır. Gerekçe: sohbette her tur tüm bağlam yeniden gönderilir (çok pahalı); kartlar tek büyük üretim çağrısında yazılır, görüntüleme koddan gelir (0 token), pahalı açıklama yalnız gerçek sorulduğunda ve dar kapsamda açılır.

**Kart formatı** (A1, karar kilidi çağrısının aynısında üretir — ekstra çağrı yok):

```
D2  Web framework: axum
    Neden        : ekosistem aktif, tokio ile doğal uyum (1-2 cümle)
    Alternatifler: actix (elendi: öğrenme eğrisi), rocket (elendi: async ekosistemi geride kaldı)
    Maliyet etkisi: nötr | Kullanıcıya etkisi: dağıtım tek dosya
    SSS          : en olası 1 kullanıcı sorusu + kısa cevap (aynı yanıtta öngörülür)
    (Depo        : SSS cevapları depoda tutulur — sorular koddan cevaplanır,
                   LLM yalnız düzenleme yapıldığında çalışır)
```

**Onay akışı (v2.16; özel komut dili yok — basit Evet/Hayır + serbest yazı):**

Kartlar gösterildikten sonra sistem tek soru sorar:

    "Bu kararlar uygun mu? [Evet / Hayır]"

- **Evet** → tümü kilitlenir, `SpecValidated`'a geçilir.
- **Hayır → açıklama alanı (v2.18):** Kullanıcı "Hayır" seçtiğinde sistem (koddan, LLM'siz) bir **açıklama alanı açar**: *"Neden olmadı, ne değişsin? Serbest yazabilirsin."* Kullanıcı gerekçesini ve istediği değişikliği yazar (girdi ucuz ve serbesttir, §14.1 kural 6). A1 bu açıklamayı girdi olarak alır, düzenlemeyi yapar ve sonucu yine **kısa kart formatında** sunar: "Böyle uygun mu? [Evet / Hayır]". Döngü Evet'e kadar sürer; her turda kullanıcının açıklaması ADR revizyon gerekçesi olarak kayda geçer. Kartla ilgili hazır sorular depodan cevaplanır — LLM yalnız düzenleme yapıldığında çalışır.
- Kart hakkında soru sormak için özel komut gerekmez: kullanıcı doğal sorusunu yazar ("rocket neden değil?") → cevap **yalnız o kartın bağlamıyla**, dar kapsamlı üretilir; standart sorular kartın SSS alanında zaten hazırdır.

- Bu ekran **gate:spec onayının teknoloji bölümüdür** — onaylanmayan kart kilitlenmez, `Advising`'de kalır.
- Token matematiği: sohbet ≈ her turda tam bağlam tekrarı; kart ≈ 1 büyük üretim çağrısı + 0 maliyetli görüntüleme + nadiren dar kapsamlı mini sorgular. Açıklama kalitesi düşmez — gerekçe/alternatif/SSS zorunlu alan olduğu için serbest sohbetten daha tutarlıdır.
- Aynı kart formatı §5.5'teki öneri cevaplarında da kullanılır (etki raporu = kart).

---

## 6. ORKESTRASYON — İKİ KATMANLI MODEL (v1 çelişkisinin çözümü)

### 6.1 Katman 1: Spec yaşam-döngüsü makinesi (proje düzeyi, deterministik)

v1'deki doğrusal makine korunur ama anlamı netleşir: **makine run'ın fazını tutar; görevlerin paralelliğini değil.**

```
Idle → SpecReceived → SpecValidated → [gate:spec] → TasksCreated → [gate:task_plan]
→ Scheduling → Finalizing → FinalReview → [gate:delivery] → Delivered
ara durumlar: Researching/Coding/Testing/... = zamanlayıcının "şu an baskın faz" bildirimi
hata yolları: Crisis (replan) | Failed | NeedsUser | Paused
```

| Mevcut | Olay | Sonraki | Sorumlu |
| --- | --- | --- | --- |
| Idle | Spec gönderildi | SpecReceived | A1 |
| SpecReceived | `spec_form.md` dolduruldu + koddan ön-kontrol (§5.4) | Clarifying | A1 |
| Clarifying | Formun belirsiz alanları için ≤3 ek soru çözüldü | Researching | A1 |
| Researching | Benzer-eser + rakip + risk araştırması tamam, öneri listesi hazır | Advising | A1 |
| Advising | Öneriler kabul/ret edildi; kararlar + bütçe + MVP kilitlendi | SpecValidated | A1 |
| SpecReceived | Form eksik/hatalı; ek soru hakkı tükendi | NeedsUser (A1 → Kullanıcı) | A1 |
| SpecValidated | Kapı onayı | TasksCreated | A2 |
| TasksCreated | Görev ağacı doğrulandı + etiketlendi | Scheduling | Zamanlayıcı + A10 |
| Scheduling | Tüm görevler son duruma geldi | Finalizing | Zamanlayıcı |
| Finalizing | Rapor hazır | FinalReview | A3 |
| FinalReview | `spec_match_pct == 100` | gate:delivery → Delivered | A1 |
| FinalReview | Uyumsuz | Crisis → A2 replan | A1→A2 |
| Crisis | Replan koşumu bitti → FinalReview | FinalReview | A2 |
| FinalReview | Uyumsuz (crisis_count ≥ 2) | **NeedsUser** — karşılanamayan gereksinim(ler) kapsam kararı olarak kullanıcıya (kapsam küçült / vazgeç / elle müdahale) | A1 |
| herhangi | Bütçe sert sınırı | **Paused(budget)** — kullanıcı bütçe yükseltip resume eder ya da suggest ile kapsamı küçültür; yükseltilmezse run Failed(BUDGET_EXCEEDED) | A10 |
| herhangi | SIGINT | Paused (kontrol noktası) → resume | Kontrol düzlemi |

**v1'den kalma ve FSM'den çıkarılmış satırlar (Coding→SecurityChecking→Testing):** o akış artık **görev hattı** şablonudur (§6.3) ve FSM'i kirletmez.

### 6.2 Katman 2: Görev zamanlayıcı (DAG, paralel gruplar)

- `tasks.json` bir DAG'dır: `dependencies` kenarlarıyla; `parallel_groups` yalnızca **bilgi amaçlıdır** (zamanlayıcı topolojik sırayı kendisi bulur).
- İşçi havuzu: `max_concurrency` (varsayılan 2, config). Bir görev yalnızca tüm bağımlılıkları `done` olduğunda `in_progress` olur.
- **Tek yazıcı kuralı:** aynı spec içinde kod yazan görevlerin `artifacts_expected` dosya yolları çakışıyorsa sıraya dizilir (çakışma tespiti zamanlayıcıdadır).
- Görev son durumları: `done | failed | skipped | blocked` — FSM'i tetiklemez; yalnız `Scheduling → Finalizing` geçişini tetikler (tümü son durumda + en az bir `done`).
- Kritik yol: `critical_path` A2'nin önerisidir; doğrulanmazsa zamanlayıcı yeniden hesaplar (öneri, gerçeklik değil).

### 6.3 Görev hattı şablonları (kod-tipi görevler)

```
code (A5) → security (A7) → test (A4) → [hata → A5 düzeltme döngüsü] → done
```
- Fix loop: max 3 iterasyon (retry sayacı `TaskFailed` olaylarında), sonra escalation ladder (§8).
- docs/report görevleri pipeline'sız tek adımdır.

### 6.4 İzlenebilirlik (traceability — v2'de zorunlu kural)

- Her `requirements[].id` **en az bir** task'ın `spec_requirement_ids`'inde olmalı → yoksa `TasksCreated` geçişi reddedilir (`TASK_UNCOVERED_REQUIREMENT`).
- Her `acceptance_criteria[].id` bir task `acceptance`'ına veya FinalReview denetimine bağlanmalı.
- **A4 test-edilebilirlik damgası (v2.4):** scheduling'den önce A4 her AC'yi denetler; ölçülemez kriter → `SpecRevised` ile geri. A1'in kâğıt üstünde kalan kriterleri schedule edilemez.
- `ComplianceCheck` bu haritayı üretir (R1→T2→`src/routes.rs:12` zinciri raporda görünür).

---

## 7. DURUM, KALICILIK VE OLAY MODELİ

### 7.1 Tek gerçeklik kaynağı (v1 çelişkilisi çözümü)

- `state.db` **yalnızca** `.ribozym/state.db` konumundadır — globaldir, tüm run'lar tek dosyada tutulur.
- `runs/<spec_id>/` altında `state.db` **olmaz** (v1'deki ikinci kopya silindi); runs klasörü yalnız insan-okur dosyalar tutar (JSON/MD/JSONL). SQLite'a tek yazar kontrol düzlemidir; JSON dosyalar onun yansımalarıdır (kontrol noktasında yazılır, okuma için değildir).

### 7.2 `state.db` şeması

```sql
CREATE TABLE runs(
  spec_id TEXT PRIMARY KEY, run_id TEXT NOT NULL, status TEXT NOT NULL,   -- FSM state
  autonomy_level INT, budget_usd REAL, spent_usd REAL DEFAULT 0,
  created_at TEXT, updated_at TEXT);
CREATE TABLE tasks(
  spec_id TEXT, task_id TEXT, status TEXT NOT NULL, agent TEXT,
  retry_count INT DEFAULT 0, escalation_level INT DEFAULT 0,
  PRIMARY KEY(spec_id, task_id));
CREATE TABLE ledger(                       -- operation ledger (§7.4)
  op_id TEXT PRIMARY KEY, kind TEXT,       -- agent_call | shell_exec | git_op | web_fetch
  spec_id TEXT, task_id TEXT, payload_hash TEXT,
  state TEXT,                              -- started | finished | failed
  result_ref TEXT, started_at TEXT, finished_at TEXT);
CREATE TABLE model_usage(
  id INTEGER PRIMARY KEY, spec_id TEXT, task_id TEXT, agent TEXT,
  provider TEXT, model TEXT, tokens_in INT, tokens_out INT, cached INT,
  cost_usd REAL, ts TEXT);
CREATE TABLE task_scores(                   -- A10 tier list + canlı skor (v2.41; §5.15, §10.3)
  model TEXT, task_type TEXT,
  bench_score REAL, success_rate REAL, tokens_per_success REAL,
  avg_cost REAL, tier_rank INT, last_used TEXT,
  PRIMARY KEY(model, task_type));
CREATE TABLE approvals(
  spec_id TEXT, gate TEXT, decided_by TEXT, decision TEXT,
  note TEXT, ts TEXT, PRIMARY KEY(spec_id, gate));
CREATE TABLE web_cache(
  key TEXT PRIMARY KEY, finding_json TEXT, fetched_at TEXT, ttl_s INT, verified_by TEXT);
```

### 7.3 Event envelope (events.jsonl — v1'den farkı: sıralama + korelasyon)

Her satır:

```json
{"seq": 41, "ts": "2026-09-11T22:06:00Z", "spec_id": "spec-20260911-001",
 "run_id": "run-9f3a", "task_id": "T1", "event": "TaskStarted",
 "correlation_id": "op-77b1", "schema_version": "1.0", "data": {"agent": "A5"}}
```

- `seq`: run içinde artan sıra numarası — resume ve tekrar-kayıt tespiti için.
- `correlation_id`: ilgili `ledger.op_id` — "bu olay hangi işlemden doğdu" sorusunun cevabı.
- Yazan tek (kontrol düzlemi); her son olayda + kontrol noktalarında diske senkronize edilir, akışta sıra korunur.

### 7.4 Resume protokolü (v1 belirsizliğinin çözümü)

1. Başlatmada: `ledger`'da `state='started'` kalmış kayıtlar taranır.
2. Yarım kalmış işlemler:
   - `shell_exec` → çıktı yeniden üretilebilir; komut tekrar çalıştırılır (araç komutları tekrar-edilebilirdir: cargo/git/docker tümü böyle).
   - `agent_call` → `result_ref` varsa atlanır (iş bitmiştir, yalnız yazma kesilmiştir → sonuç tekrar yazılır); yoksa aynı prompt+girdi hash'i ile tekrar denenir (aynı girdiyle yeni deneme kabul edilebilir).
   - `git_op` → SHA kontrolü: push hedefte varsa atlanır.
3. Sonra FSM, `tasks` tablosundaki son durumlarla senkronize edilir; zamanlayıcı kaldığı yerden devam eder.
4. Resume **asla** bitmiş görevi tekrar çalıştırmaz — kanıt `ledger.finished` kaydıdır.

**Kesin yazım sırası (v2.47 B4 — atomiklik penceresini küçültme kuralı):** her işlem için yazım sırası sabittir: (1) `ledger INSERT state='started'` → (2) `state.db` transaction (task durum değişimi) → (3) `events.jsonl` append → (4) `ledger UPDATE state='finished'`. Çökme hangi adım arasında olursa olsun resume protokolü kapanışı yapar: "olay yazıldı + ledger finished yok" penceresindeki işlemler, yukarıdaki tür-bazlı kurallarla (2. madde) idempotent şekilde tamamlanır ya da tekrar edilir. Bu pencere golden resume testinin zorunlu varyantıdır (§21.2).

### 7.5 Eşzamanlılık ve kilit

- Tek `ribozym` kopyası varsayımı: `.ribozym/lock` (pid + değişim zamanı). Çökmüş kilit (pid yok) otomatik kurtarılır.
- SQLite WAL + busy_timeout 5000 ms. Veritabanına yazan yalnız kontrol düzlemidir; ajanlar dolaylı (olay yolu üzerinden) yazar.

### 7.6 Arşiv katmanı (v2.22; kullanıcı kararı)

- **Konum:** `runs/<spec_id>/archive/` — her projenin kendi dizininde, projeye ait her şey.
- **Mekanizma:** her aşama/olay tamamlandığında çıktı koddan **zstd** ile sıkıştırılıp bölüm numarasıyla arşivlenir: `001-mulakat.zst`, `002-analiz.zst`, `003-decision.zst`... Arşivleme **arka planda** yapılır (run'ı geciktirmez); hızlı sıkıştırıcı + ayarlanabilir seviye = düşük donanım yükü.
- **Tutarlılık kaynağı (Risk 1'in çözümü):** her ajan/aşama çağrısına bağlam **arşivden verilir** — sonraki aşama önceki aşamanın arşivlenmiş çıktısını okur; karar kanıtları arşivle eşleştirilir. Hafıza konuşmada değil, arşivdedir ("her sabah yeni işçi" + elinde dünkü tutanaklar).
- **Resume kaynağı:** hızlı resume ledger/events (SQLite) üzerinden yapılır; arşiv **tam içerik yedeğidir** — insan okuyabilir, derin bağlam gerektiğinde açılır. "Nerede kaldı → tam kaldığı yerden devam" hem ledger hem arşivden doğrulanabilir.
- **Git:** `runs/` `.gitignore`'dadır — arşiv GitHub'a gönderilmez, yalnız yerelde yaşar.
- **Bütünlük manifest'i (v2.47 B2):** her arşiv bölümü yazılırken sha256 özeti `runs/<spec_id>/archive/manifest.json`'a işlenir (bölüm adı + hash + byte boyutu + yazım ts); her okuma anında hash doğrulanır. **Bozuk/eksik bölümde sessiz-devam yasaktır:** A12 bulguyu işaretler (`ArchiveCorrupt` olayı) → bölüm yeniden üretilebiliyorsa (kaynak veri ledger/events'te yaşıyorsa) koddan yeniden yazılır, üretilemiyorsa run NeedsUser'a park eder. Arşiv, bağlamın tek kaynağı olduğu için çürümesine göz yumulmaz.

---

## 8. ESKALASYON, RETRY VE BÜTÇE

### 8.1 Escalation ladder (parametreli — v1 sadece kural metniydi)

| Kademe | Tetik | Eylem | Parametre (default) |
| --- | --- | --- | --- |
| E0 | Geçici hata (timeout, 5xx, tool busy) | Aynı ajan + aynı model, retry | max 3, backoff 5s→15s→45s (full jitter) |
| E1 | Kaliteli çıktı hatası (derlenmiyor, şema-dışı, test fail) | Deterministik fix döngüsü (A7/A4 → A5) | max 3 iterasyon |
| E2 | E1 tükendi | A2'ye eskale: replan / görevi böl / tier yükselt | 1 kez |
| E3 | Hâlâ başarısız | Bulut model tier yükseltme (cheap→strong); A10 bütçe onayıyla | 1 kez |
| E4 | Tümü tükendi | Kullanıcı raporu: `NeedsUser`, yapılandırılmış hata + deneme geçmişi | — |

Her kademe `Escalated*` olayı + `escalation_level` alanı ile izlenir. **Ücret ilkesi:** her ajan varsayılan olarak *ucuz* modelden başlar; *strong* model yalnız E3'te ve A10 bütçe onayıyla kullanılır (P4).

### 8.2 Bütçe hiyerarşisi (v1 çelişkisi çözüldü: $1.50 vs $5)

```
run bütçesi  ≤ spec.budget.max_cost_usd          (varsayılan 1.50 — spec bazlı, A1 yazar)
aylık bütçe  ≤ ribozym.toml budget.monthly_usd  (varsayılan 5.00 — sistem bazlı, A10 izler)
```

Kontrol noktaları (hepsi A10'da: çağrı **öncesi** tahmini + **sonrası** fiili):

1. **Ön-kelepçe:** her bulut çağrısının tahmini maliyeti = `(in/1M)·in_price + (out/1M)·out_price` (girdideki prompt + tipik çıktı uzunluğundan hesaplanır).
2. `spent + est > soft_limit(80%)` → `BudgetWarning` olayı; model seçimi fiyata kayar (§10.3), benchmark/keşif harcaması durur.
3. `spent + est > hard_limit(100%)` → bulut çağrısı reddedilir, yerel modele yönlendirilir; yerel de yoksa run **`Paused(budget)`** olur (v2.47 B3 — doğrudan `Failed` değil): kullanıcı bütçeyi yükseltip `resume` eder ya da suggest kartıyla kapsamı küçültür; hiçbiri yapılmazsa run `Failed(BUDGET_EXCEEDED)` ile sonlanır. Bitmiş görevler ve ledger durumu korunur — resume kaldığı yerden devam eder.
4. Aylık sayaç `.ribozym/state.db`'de kalıcıdır; spec'ler arası ortaktır.
5. Fiyat teyidi: fiyatlar **arka planda 5 dakikada bir** OpenRouter API'den çekilip önbellekte tutulur (v2.43) — seçim anında önbellekten okunur, çağrı içinde ekstra fiyat isteği yapılmaz. Çekim başarısız olursa son bilinen fiyat + %15 güvenlik payıyla devam edilir ve `PRICE_STALE` olayı yazılır (§5.2) — bu da yoksa (model hiç ölçülmemişse) çağrı reddedilir.
6. **Kalite-takas şeffaflığı (v2.47 B3):** %80 sonrası fiyat-lehine model kayması kalite düşüşü riski taşır; bu takas sessiz kalamaz — etkilenen görevler raporun `quality_tradeoff` alanına (görev, seçilen model, düşen tier, gerekçe) yazılır ve FinalReview denetiminde göz önündedir.

---

## 9. ONAY PROTOKOLÜ (kapılar/gate'ler — v1'de hiç tanımlı değildi)

| Gate | Zamanlama | Level 1 | Level 2 | Level 3 |
| --- | --- | --- | --- | --- |
| `spec` | SpecValidated sonrası, A2'den önce | zorunlu | zorunlu | otomatik |
| `task_plan` | TasksCreated sonrası, planlamadan önce | zorunlu | otomatik | otomatik |
| `delivery` | FinalReview sonrası | zorunlu | zorunlu | opsiyonel (`--yes`) |

Mekanizma:

- **Etkileşimli:** terminal sorusu; A1'in ürettiği okunur fark/özet gösterilir; `[approve / revise / abort]`. Revise → `SpecRevised` olayı + yorum A1'e döner.
- **Toplu (`--yes`):** tüm kapılar otomatik onaylıdır, **ancak** durdurma koşulları (sert bütçe sınırı, `needs_revision` red) yine durdurur.
- **Dosya üzerinden (uzaktan/CI):** `ribozym approve <spec_id> <gate> [--note "..."]` komutu `approvals` tablosuna yazar.
- **Risk bazlı zaman-aşımı politikası (Q4 kararı):**
  - Onay **asla üretimi durdurmaz** — kapı beklenirken sistem izole alanda çalışmayı sürdürür; yalnızca sonraki *sonlandırıcı* adım (entegrasyon/push) kilitli kalır.
  - Düşük riskli kapılar (`spec`, `task_plan`): `gate_timeout_seconds` (varsayılan 1200) dolarsa **otomatik onaylanmış sayılır** ve `HumanApprovalGranted(reason="timeout_auto")` olayıyla kayda geçer. Kullanıcı sonradan revize edebilir — bu noktada ana koda dokunulmamıştır.
  - Yüksek riskli kapı (`delivery`): zaman aşımında otomatik onay **yoktur**; run `NeedsUser` olarak park edilir, hiçbir iş kaybolmaz, `resume` ile devam edilir.
- Her karar `approvals` tablosuna + `HumanApprovalGranted|Denied` olayına yazılır. **Denetim izi eksiksizdir:** kim, neyi, ne zaman, hangi notla (zaman-aşımı otomatik onayları dahil).

Otonomi düzeyi `specs.json`'da yazılıdır; `--autonomy` CLI bayrağı yalnızca **düşürebilir** (güvenli yön), yükseltemez.

---

## 10. ÇIKARIM VE MODEL SEÇİMİ

### 10.1 Sağlayıcılar

| Sağlayıcı | Uç nokta | Protokol | Not |
| --- | --- | --- | --- |
| bulut | OpenRouter | [OI]-uyumlu | **Birincil motor — tüm görevler varsayılan olarak buradan.** `governor` 10 RPM / 5 eşzamanlı; `HTTP-Referer` + `X-Title` başlıkları |
| yerel | `llama-server` (llama.cpp) | [OI]-uyumlu `/v1/chat/completions` | **Yardımcı yük paylaşımı — asla zorunlu değil.** Yalnız A10'un ucuz işaretlediği görev tiplerinde (şablon kod, basit test iskeleti, sınıflandırma) maliyet/süre kazanmak için kullanılır. VRAM kabulü: istenen bağlam + model yükü, boş VRAM'den büyükse görev sessizce buluta gider. **Model (Q1 kararı):** Qwen2.5-Coder-7B-Instruct Q4_K_M (`C:\Users\Rog\qwen_model\`, ~4.7 GB) — RTX 5050 Laptop, 8 GB VRAM |

`InferenceProvider` arayüzü ikisini de sarar; tekrar/sınırlandırıcı/loglama katmanı arayüzün dışındaki tek ortam katmanıdır.

### 10.2 Yönlendirme akışı (A10 politikası, `model_selector` mekanizması)

```
görev (task_type, metadata, DAG bağlamı) → A10 zorluk analizi + eleme + tier-list seçimi (§10.3) → (sağlayıcı, model, fiyat teyidi)
```
- **Varsayılan her görev buluttadır (v2.3 düzeltmesi):** `target: cloud` — seçici ucuz/strong'dan birini seçer.
- `target: hybrid` → görevin ucuz alt adımları (şablon kod, basit test üretimi, sınıflandırma) A10 kararıyla yerel modele **yönlendirilir**; bu bir *iyileştirmedir*, zorunluluk değildir — yerel sağlıksızsa/VRAM doluysa görev sessizce buluta gider (ceza yok, E0 sayılmaz).
- `target: local` yalnızca A10'un açıkça ucuz işaretlediği görevlerde geçerlidir; yerel çıktı şema kontrolünden geçmezse görev buluta döner.

### 10.3 Seçim: eleme + tier list + zorluk (v2.42 — A10 kendi benchmarkıyla ölçer)

```
girdi   : görev (task_type, metadata, DAG bağlamı) + kalan iş bütçesi
adım 1  : A10 görevi kendisi analiz eder → zorluk derecesi (§5.15 m2; koddan)
adım 2  : aday eleme — OpenRouter API (liste + canlı fiyat) + Artificial Analysis API (kalite)
          kriter: ucuz VE performanslı (§5.15 m3)
adım 3  : seçim = tier_list[task_type] sırasından, zorluğa uygun ilk aday
          - tahmini maliyet kalan bütçeyi aşıyorsa → sıradaki daha ucuz aday
          - A11'in arama modeliyle çakışırsa → sıradaki aday (kanıt, §13.3)
%80+ bütçe baskısı: seçim fiyat-lehine bir adıma kayar, test harcaması durur (§8.2 m2)
ölçülmemiş model: aday olamaz — önce test kotasıyla kendi benchmarkından geçmelidir (§5.15 m5)
```

- **Puan birleşimi:** tier sırası = **kendi benchmark skoru (birincil, §5.15 m4–5)** + gerçek başarı (`task_scores`, son 20 deneme). Benchmark başlangıç puanını verir, sahadan gelen veri sırayı kırar. Artificial Analysis skorları yalnız aday elemesinde kullanılır; sıralamada birincil ölçü her zaman kendi benchmarktır.
- **Dış veri ritmi (v2.43):** fiyatlar arka planda **5 dakikada bir** OpenRouter'dan çekilir, seçim önbellekten okur (§8.2 m5); **Artificial Analysis kalite skorları günde 1 kez** çekilir (skorlar saatlik değişmez; run içinde tekrar tekrar istek atmak boşa trafigtir). Her iki çekim de arka plandadır — seçim yolu asla dış API beklemez.
- **Keşif:** yeni model yalnız test kotasından ölçülerek havuza girer; çalışma bütçesinden rastgele-model denemesi yoktur (eski %5 keşif payı kuralı kalktı → config'te `model_test_quota`).
- **Çağrı-bazlı seçim korunur (v2.23):** aşama-model kilidi yoktur; her çağrı kendi görevi ve A10'un çıkardığı zorluk derecesiyle seçilir. `agents.toml`'daki `model_tier` yalnız başlangıç önerisidir.
- Kara liste kuralları: aynı model + aynı görev tipi üst üste 3 kez A4/A7'de başarısız → kara liste; A11 doğrulama modelinin orta skoru < 50 (son 10 doğrulama) → `ModelBlacklisted` + inceleme; elle çıkarma yalnız `--reason` ile.
- Skor tablosu: `task_scores` (§7.2) — `(model, task_type) → {bench_score, success_rate, tokens_per_success, avg_cost, tier_rank, last_used}`.

### 10.4 Prompt & önbellek stratejisi

- Şablon (v1 §17) korunur; **sabit önek kuralı:** `[SYSTEM]` bloğu run boyunca bayt-bayt aynıdır (sağlayıcı prompt önbelleğini tetikler); yalnız `[CONTEXT]`/`[INPUT]` değişir.
- `prompt_optimizer` arayüzü: `compress(prompt, task_type)`, stratejiler `structured|template|reference|abbreviation`, varsayılan kayıpsız `structured+template`. Kalite kontrolü: sıkıştırılmış/ham çıktı farkı kör-diff skoruyla ölçülür (A11 doğrulama yöntemi, §13.3); eşik altıysa otomatik ham metne düşülür.
- Token sayımı: her çağrıda sağlayıcının `usage` alanı **gerçek kaynaktır**; tahmini ön-kelepçe (§8.2) yalnız karar anı içindir.

---

## 11. VERİ SÖZLEŞMELERİ (şema politikası değişti)

### 11.0 Şema yönetimi (v1 eksiği)

- Her sözleşmenin resmi `schemas/*.schema.json` (draft 2020-12) dosyası `ribozym/schemas/` altında tutulur; dokümandaki JSON örnekleri **referanstır**, kural değil. LLM çıktıları çalışma anında bu şemalarla doğrulanır (jsonschema crate), `config validate` ile CI'da da sınanır.
- `schema_version: "1.0"` → major.minor. Minor: yeni opsiyonel alan (geriye uyumlu). Major: veri taşıma (migration) gerekir; SQL dosyaları `.ribozym/migrations/` altındadır, resume öncesi otomatik uygulanır.
- `deny_unknown_fields` yoktur: bilinmeyen alanlar korunur (ileriye-uyumlu).

Aşağıdaki sözleşmeler v1'den gelir; burada **yalnızca değişen/yeni alanlar** yazılıdır (tam örnekler v1 §4 ile aynıdır, şema dosyalarına taşınmıştır):

### 11.1–11.5 — v1'den gelen sözleşmeler (değişiklikler)

- `specs.json` — yeni zorunlu alanlar: `run_id`, `raw_input_hash`; `autonomy_level` + `approval_required` etkileşimi: `approval_required` yalnız otonomi düzeyini **daraltabilir**, genişletemez (§9 tablosu üst sınır). **v2.4:** yeni `open_questions[]` + `assumptions[]` (sessiz tahmin yasağı, §5.4); FinalReview varsayımları denetler. **v2.5:** girdi `spec_form.md` oldu (kullanıcı formu, şablon: `examples/spec_form.md`); spec'e `decisions[]` (danışma diyaloğunun sonucu) + `mvp_scope` alanları eklendi. **v2.6:** `decisions[]` ADR biçimine bağlandı (choice + alternatives_considered + rationale + evidence zorunlu) ve A1'in yetkisi **teknoloji kararlarını** kapsayacak şekilde genişletildi.
- `tasks.json` — `parallel_groups` "yalnız bilgi" olarak işaretlendi (§6.2); yeni: `file_ownership: [paths]` (dosya çakışmasını sıraya dizmek için, §6.2).
- `ExecutionResult` — `model_usage` boş (null) olabilir (A7 tamamen koddan çalışır; A9/A10'un LLM yollarında doludur, kural/şablon yollarında boştur — §5.14 m2, §5.15). Yeni: `idempotency: {op_id, input_hash}` (resume için §7.4).
- `events.jsonl` — zarf alanları zorunlu: `seq`, `run_id`, `correlation_id` (§7.3).
- `StandardDepartmentReport.json` — yeni: `traceability` bölümü (gereksinim→görev→kanıt haritası, §6.4); `compliance.spec_match_pct` yalnız 100'de `Delivered` mümkündür (P5).

### 11.6 `models.toml` — v1 ile aynı; ekleme

```toml
[[model]]
id = "..."
task_types = ["code", "plan", "verify", "report"]  # yeni: görev tipine göre etiket
context_limit = 131072                              # yeni
```

### 11.7 `agents.toml` — **YENİ (v1'de dosya vardı, şeması yoktu)**

```toml
[[agent]]
id = "A5"
title = "Lead Coder"
prompt_file = "config/prompts/A5_coder.md"
model_tier = "cheap"        # low | medium | high | none (tamamen koddan çalışan ajanlar)
escalation_tier = "strong"  # E3'te çıkacağı düzey
max_concurrency = 1         # bu ajanın aynı anda taşıyabileceği görev sayısı
budget_share_pct = 40       # bulut bütçesinden üst sınır payı
tools = ["cargo.check", "cargo.test", "fs.write:workdir"]   # §12.3 referansı
```

### 11.8 Yeni çıktı sözleşmeleri

`ComplianceCheck` (A1 çıkış denetimi — `delivery` kapısının veri kaynağı):

```json
{ "spec_id": "...", "checked_by": "A1", "model": "...",
  "spec_match_pct": 100, "requirements": [
    {"id": "R1", "status": "met", "evidence": "src/routes.rs:12", "test_id": "T4#AC1"}],
  "decisions_compliance": [
    {"decision_id": "D2", "title": "Web framework: axum", "status": "met", "evidence": "Cargo.toml deps"}],
  "unmet_requirements": [], "verdict": "approved" }
```

`WebQuery` / `WebFinding` / `VerificationScore` → §13.

### 11.9 Ajan-ajansı iletişim zarfı (v2.30; kullanıcı kuralı)

Tüm ajanlar arasındaki iletişim **tek standart zarfla** taşınır — serbest LLM sohbeti yoktur. Zarf her zaman **Rust çekirdeğinden** (kontrol düzlemi) geçer: hızlı, denetlenebilir, token-ekonomik.

```json
{ "from": "A2", "to": "A5", "type": "task_assignment",
  "spec_id": "spec-...", "task_id": "T3", "seq": 42, "correlation_id": "op-...",
  "ts": "2026-09-12T10:00:00Z", "schema_version": "1.0",
  "payload": { "brief": "...", "constraints_ref": "..." } }
```

- **Sabit alanlar** (from/to/type/ts/seq/ids) **koddan** dolar; LLM yalnız payload içinde **üretilmesi gereken alanları** yazar (§14.1 çıktı ekonomisi). İletişim sıklığı ne olursa olsun çıktı token'ı fırlamaz.
- LLM çıktısı asla doğrudan iletişim değildir — önce şema kontrolünden geçer, sonra zarfa konur (§3.1: LLM önerir, kontrol düzlemi taşır).
- Her zarf olaylarla/ledger ile ilişkilidir (§7.3) — tüm iletişim tamamen izlenebilir ve resume-tutarlıdır.

---

## 12. ARAÇ KATMANI VE GÜVENLİK (izolasyon)

### 12.1 İlke

Hiçbir ajan alt süreç (subprocess) doğrudan çalıştıramaz. Tek kapı: `ToolGateway` (kontrol düzlemi). Geçiş sırası: **normalize → izin listesi → izole ortam → çalıştır → çıktı temizliği → kayıt**.

### 12.2 Komut normalizasyonu

- Komutlar `argv` biçiminde çalıştırılır (shell yok → enjeksiyon yolu kapalı). `"cargo test -- --exact name"` gibi argüman ayrımı araç tanımında yapılır.
- Normalizasyon: boşluklar, mutlak→göreli yol; izin listesi eşleşmesi önek değil **tam argüman vektörü** olarak yapılır (örn. `cargo check` ✔, `cargo check --config build.rs=...` ✘ — bilinmeyen genel bayrak reddedilir, araç tanımının izin verdikleri hariç).

### 12.3 Allowlist (v1'den; sıkılaştırma)

```toml
# ribozym.toml
[toolchains.rust]
allowed = ["cargo check", "cargo test", "cargo clippy", "cargo fmt", "cargo audit", "cargo build --release"]
# ağ erişimi yok; izin listesinde olmayan her komut → TOOLCHAIN_NOT_ALLOWED olayı + ret
fs_read  = ["workdir/**", "runs/<spec_id>/artifacts/**"]
fs_write = ["workdir/**"]            # asla ~/.ssh, /etc, repo dışı
env_scrub = "deny"                   # alt sürecin ortamı: *_KEY/*_TOKEN/*_SECRET silinir, PATH/HOME/LANG kalır
timeout_s = 300                      # komut bazında geçilebilir
output_cap_kb = 1024                 # aşılırsa kırpılır + olay yazılır
```

### 12.4 Ön-kontrol (run başlangıcı)

`ribozym run`, iş başlamadan önce `agents.toml`'da istenen tüm araçları sınar (`cargo --version`, `git --version`, docker opsiyonel). Eksikse → **hemen** `TOOLCHAIN_MISSING` raporu, run başlatılmaz. (v1'de bu hata run'ın ortasında patlayacaktı.)

### 12.5 Git/GitHub güvenliği

- `gh` kullanımı izin listesine bağlıdır; push **yalnızca** `delivery` kapısından sonra (A8 — §5.13 m7); dal koruması: sistem `main`'e push atmaz, her zaman `ribozym/<spec_id>` dalı + PR açar.
- Gizli anahtar taraması A7'de zorunlu adımdır: `gitleaks`-tarzı regex + `cargo` bağımlılık taraması; bulgu varsa run ilerlemez (`SEC_SECRET_FOUND`).

### 12.6 Mevcut projeye çalışma (brownfield) izolasyonu (Q3 kararı)

- Sistem mevcut projeye (`target_path`) **asla doğrudan yazmaz**: çalışma öncesi proje `runs/<spec_id>/workdir` altına kopyalanır (tercihen konteyner içi volume; alternatif dosya kopyası) — tüm değişiklik izole alanda olur, orijinal proje her koşulda güvende kalır.
- Entegrasyon kalite kapısı: projenin **mevcut test paketi** izole alanda koşulur; test düşerse A5 düzeltme döngüsüne girer; geçmeden teslim yoktur.
- Teslim: fark/patch + `ribozym/<spec_id>` dalı + PR; yalnızca `delivery` kapısı onayıyla orijinal projeye uygulanır.
- "Neye göre ekleme" üçlüsü: spec gereksinimleri (ne eklenecek) · repo yapısı/tarz analizi A5 (nereye, hangi tarzda) · mevcut testler (bozmadı kanıtı).

---

## 13. WEB ERİŞİMİ VE KÖR DOĞRULAMA (A11 — iki rol, iki model; v2.45)

### 13.1 A11 — tek internet kapısı

- Tüm harici **içerik** yalnız A11'den geçer; diğer ajanlar A11'i `WebQuery` ile çağırır. Muafiyet (v2.41): kontrol düzleminin teknik bağlantıları — model çağrıları, fiyat teyidi, benchmark sağlayıcı API'leri — bu kapıya girmez; A11 yalnız içerik aramasının tek kapısıdır.
- A11'in iki rolü vardır (v2.45): **arama** (agent-reach birincil backend, aşağıda) ve **doğrulama** (kör değerlendirme, §13.3). Doğrulama internet gerektirmez — yalnız arama sonucu verisiyle çalışır, dış bağlantı açmaz; tek internet kapısı ilkesi bozulmaz.
- Arama backend'i takılabilir: `WebFetcher` arayüzü — **MVP'de (Q2/Q5 kararı) agent-reach birincil** (kullanıcı makinesinde kurulu, `fetcher = "agent-reach"`), yedek: DuckDuckGo HTML (anahtarsız), Tavily opsiyonel (v1.0).
- Arama çağrısı strong model kullanır; doğrulama çağrısı **farklı bir model** kullanır (seçici dışlama kuralı, §10.3); A10 bütçe kelepçesi ikisine de geçerlidir.

### 13.2 Sözleşmeler

```json
// WebQuery (girdi)
{ "id": "q-01", "ask": "axum 0.7 streaming response example", "require_fresh": false, "min_sources": 2 }
// WebFinding (çıktı, çoklu kaynak zorunlu)
{ "query_id": "q-01", "claim": "...", "sources": [{"url": "...", "domain": "docs.rs", "type": "docs", "retrieved_at": "...", "snippet_hash": "sha256:..."}],
  "injection_flagged": false }
```

Önbellek: `.ribozym/state.db` içindeki `web_cache` tablosu; anahtar = `sha256(normalize edilmiş soru + sağlayıcı)`; süre sınırı (TTL) varsayılan 24 saat. Önbelleğe alınmış sonuç yeniden doğrulanır mı? Hayır — `verified_by` alanı önbelleğe yazılmıştır; süre içinde aynı iddia yeniden puanlanmaz.

### 13.3 A11'in ikinci rolü — kör doğrulama protokolü (eski A12; v2.45'te aynı ajana birleşti, kurallar değişmedi)

**Neden ayrı ajan değil ama ayrı model (v2.45, kullanıcı kararı):** doğrulama, arama çağrısından **yapısal olarak ayrı** bir LLM çağrısıdır — modeli farklı olmak zorundadır, girdisi kördür; teklik kaybı yoktur, yalnız sahiplik tek elde toplanmıştır. Dört-göz ilkesi veri yoluyla korunur, ajan sayısıyla değil.

Girdi: iddia + kaynaklar + zaman damgası **yalnız** — `VerificationRequest` tipinde üretici alanı yoktur; ham sayfa içeriği ve arama bağlamı asla girmez (tip düzeyinde körlük korunur).

**MVP notu (Q2 kararı):** v0.1'de yalnız **kural katmanı** çalışır (`confidence` kural skorundan türetilir, çıktıda `rules_only: true` işaretlidir); LLM kör puanlama katmanı v1.0'da açılır.

İki katmanlı skor → tek `confidence`:

1. **Kural katmanı (koddan):** farklı alan adı sayısı ≥ 2 (+), kaynak tipi ağırlığı (docs/gov/edu=1.0, blog=0.4), yaş ≤ 24 ay, kaynaklar arası çelişki taraması.
2. **LLM katmanı (farklı model):** "iddia kaynaklarla örtüşüyor mu?" → destekleme puanı 0–100 + itiraz listesi.

`confidence = round(0.4·kural + 0.6·llm)` → **high ≥ 75 | medium 50–74 | low < 50**.
Davranış: `low` → iddia atılır, A6'ya "unverified" olarak gider; `medium` → context.md'de etiketlenir; `high` → serbest kullanılır. Arama↔doğrulama uyuşmazlık oranı > %40 (son 20) → arama modeli inceleme listesine girer.

---

## 14. PROMPT SİSTEMİ

v1 §17 şablonu + ek kurallar:

- Her ajanın prompt dosyası `config/prompts/<A_id>_<role>.md`; ilk satırda `model_tier` ve `output_schema` referansı vardır (çalışma anında şema bağlanır).
- Şemaya uymayan LLM çıktısı **tek seferlik** düzeltme turuyla kurtarılmaya çalışılır (koddan çözümleyici: JSON ayıklama + şema fark mesajı); ikinci hatada E0 sayacı işler.
- Önbellekleme: OpenRouter prompt caching — önek sabittir (§10.4); önbellek kazancı `cache_savings_usd` alanında muhasebeleşir (raporda görünür).

### 14.1 Çıktı ekonomisi (v2.14; kullanıcı ilkesi: "girdi ucuz, çıktı pahalı — çıktıyı iyi kurgula")

LLM fiyatlamasında girdi ucuzdur (sabit önek önbelleklenince neredeyse bedava); **çıktı pahalıdır** (birim fiyat 3–5 kat, üretim süresi en yavaş). Maliyeti belirleyen girdi boyutu değil, çıktının nasıl üretildiğidir. Kurallar:

1. **Tek üretim, çok kullanım:** Model her çıktıyı bir kez üretir (karar kartı, özet, açıklama) → dosyaya yazılır → her ekranda koddan gösterilir. Yeniden görüntüleme = 0 token.
2. **Kısa ve yapılandırılmış çıktı varsayılandır:** Aynı bilgi 3 alanlı kartta (Neden/Alternatif/Etki) hem daha az token ile üretilir hem kullanıcıya daha net anlatılır. Uzun serbest yazı istisnadır (yalnız `executive_summary` gibi açıkça belirtilmiş alanlarda).
3. **Tekrarlayan kalıplar LLM'e yazdırılmaz:** Kart çerçevesi, tablo düzeni, rapor iskeleti koddan gelir; LLM yalnız değişen içerik alanlarını üretir.
4. **Öngörülebilir içerik aynı turla üretilir:** SSS, uyarılar, olası itirazlar — ileride ekstra tur açmamak için aynı üretim yanıtının içinde yazılır (§5.4B SSS alanı bu modelin örneğidir).
5. **Büyük girdi + küçük yapılandırılmış çıktı:** Tüm prompt şablonları "kompakt ve yapılandırılmış yanıt ver; girdiyi asla tekrarlama" talimatıyla biter. Doğrulama: `tests/` içinde çıktı boyutu sınırı testi (çıktı başına üst sınır).
6. **Asimetrik iletişim (v2.15):** İki yönün maliyeti farklıdır, davranış da öyledir. **Kullanıcı girdisi her zaman serbest biçimdedir** — tek cümle, sayfalarca not, dağınık sohbet; format istenmez, düzeltilmez (girdi ucuz, formatı düzenlemek sistemin işidir). **AI çıktısı her zaman kısa ve sabit formatlıdır** — planlamanın her temasında (soru açıklaması, analiz özeti, öneri cevabı, karar kartı) aynı kart biçimi kullanılır; kullanıcı formatı bir kez öğrenir, her yerde aynıdır. "Kısa ama çok faydalı" tercih değil, zorunlu kuraldır.
7. **Tek format ailesi (v2.15):** Kart formatı (alan adları: Ne / Neden / Alternatif / Etki / SSS) tüm planlama çıktılarında ortaktır — §5.4B karar kartları, §5.5 öneri cevapları ve Aşama 2 analiz açıklamaları aynı şablonu kullanır; farklı içerik = farklı alan değerleri, farklı format değil.
8. **Depo-önce ilkesi (v2.17):** Her kullanıcı etkileşiminde önce önceden üretilmiş içerik (kart + genişletilmiş detay + SSS) koddan sunulur; sabit yönlendirme metinleri ("lütfen açıklayın", "hangi kart?") koddur — token değildir. LLM yalnız depoda olmayan gerçekten yeni içerik için çalışır.

---

## 15. GÖZLEMLENEBİLİRLİK

- `tracing` JSON log → `.ribozym/logs/ribozym-<tarih>.log` (döner dosya) — **teknik hata/uyarı**; iş olayları `events.jsonl`'dadır (durum makinesinin gerçeği). İkisi karıştırılmaz.
- `ribozym status [spec_id]`: tek ekranda — FSM fazı, görev ilerleme çubuğu (ajana göre done/toplam), bütçe `harcanan/limit`, aktif görevler + token hız, son 5 olay.
- `--follow`, `attach`: `events.jsonl` izleme + durum; `attach` yalnız izler, müdahale etmez.
- `metrics.json` (runs altında): toplam token, maliyet, tekrar denemeleri, eskalasyonlar, önbellek isabet oranı → §22 faz raporlarına veri.

---

## 16. HATA SINIFLANDIRMASI (tam tablo — v1'de yalnız önek listesi vardı)

| Kod | Önek | Önem | Kurtarılabilir | Varsayılan eylem |
| --- | --- | --- | --- | --- |
| `SPEC_PARSE_FAILED` / `SPEC_INVALID` / `SPEC_UNAPPROVED` | SPEC | hata | hayır | Kullanıcıya düzeltme sorusu (E4) |
| `TASK_UNCOVERED_REQUIREMENT` / `TASK_CYCLE_DETECTED` / `TASK_LOOP_EXCEEDED` | TASK | hata | kısmen | A2 replan (E2) |
| `TOOLCHAIN_MISSING` / `TOOLCHAIN_CHECK_FAILED` / `TOOLCHAIN_NOT_ALLOWED` | TOOLCHAIN | hata | evet(eksik araç)/hayır(izin) | A5 düzeltme döngüsü / güvenlik ret |
| `MODEL_SCHEMA_INVALID` / `MODEL_UNAVAILABLE` / `MODEL_BUDGET_DENIED` | MODEL | uyarı–hata | evet | düzeltme turu → E0 → model değişimi |
| `BUDGET_SOFT_LIMIT` / `BUDGET_EXCEEDED` | BUDGET | uyarı / ölümcül | hayır | bulut→yerel / run `Failed` |
| `NETWORK_TIMEOUT` / `NETWORK_PROVIDER_429` | NETWORK | uyarı | evet | E0 bekle-yeniden dene |
| `APPROVAL_DENIED` / `APPROVAL_TIMEOUT` | APPROVAL | bilgi / hata | hayır | NeedsUser |
| `SEC_SECRET_FOUND` / `SANDBOX_VIOLATION` | SEC | ölümcül | hayır | run durdur, raporla |

Hata JSON'u (v1 §16 biçimi) her olayın `data.error` alanına gömülür; `context` içindeki stderr **kırpılmıştır** (output_cap).

---

## 17. RAPORLAMA VE TESLİMAT

- A3 `StandardDepartmentReport.json` üretir (v1 §4.6 + izlenebilirlik §11.5); yönetici özeti ≤ 10 satır, `executive_summary` içinde maliyet/süre/uyum tek paragrafta.
- Çift imza zinciri: A3 raporu (üretim) + A1 `ComplianceCheck.verdict=approved` (denetim) + gate:delivery (kullanıcı) → `Delivered`. Eksik imza → teslim yok.
- **Karar uyumu denetimi (v2.6):** `ComplianceCheck` ayrıca kodun ADR kararlarına uyumunu denetler. **Öz-denetim riskine karşı** uyum, A7/A4'ün ürettiği kanıtla ölçülür (bağımlılık manifestosu, import taraması, test envanteri) — A1'in yargısal "uyuyor" beyanı tek başına kabul edilmez.

---

## 18. YAPILANDIRMA REFERANSI

```
~/.ribozym/config.toml   (kullanıcı-genel)  ← öncelik: düşük
<repo>/ribozym.toml      (proje)            ← orta
config/agents.toml, models.toml              ← ajan/model kayıt defteri
.env / ortam değişkenleri (RIBOZYM_*, *_API_KEY) ← yüksek (anahtarlar yalnız burada)
CLI bayrakları                               ← en yüksek
```

- `ribozym config validate`: şema + izin listesi + tier tutarlılığı + anahtar sızıntı taraması (config dosyalarında da) → CI'da çalışır.
- `.env.example`: `OPENROUTER_API_KEY`, `AA_API_KEY` (opsiyonel), `LLAMA_SERVER_URL` (varsayılan `http://127.0.0.1:8080`).

---

## 19. DİZİN VE REPO YAPISI

Repo yapısı v1 §8 ile aynı; değişiklikler:

```
ribozym/
├── schemas/                    ← YENİ: *.schema.json (draft 2020-12), §11.0
│   ├── spec.schema.json
│   ├── tasks.schema.json
│   ├── execution_result.schema.json
│   ├── department_report.schema.json
│   ├── event.schema.json
│   ├── compliance_check.schema.json
│   └── web_query.schema.json / web_finding / verification_score
├── .ribozym/migrations/       ← YENİ: 0001_init.sql ...
├── src/prompt_optimizer/       ← korundu (MVP arayüzü, algoritmalar v1.1'de)
├── config/questions/           ← YENİ (v2.9/v2.11): genel soru seti (tek, sabit core.md) + bağlama-uyarlama prompt'u (§5.4 Aşama 1)
└── examples/
    ├── spec_form.md            ← kullanıcı giriş formu şablonu (§5.4, `ribozym new-form` üretir)
    ├── spec_rest_api.md        ← golden run girdisi (§21.3)
    └── expected/                ← golden çıktı karşılaştırma dosyaları (insta)
```

Çalışma dizinleri: `runs/<spec_id>/` yalnız üretim dosyaları + JSON yansımaları; **state.db ve önbellekler `.ribozym/` altındadır**. `.gitignore`: `.env`, `.ribozym/`, `runs/`, `target/`, `*.key`, `*.pem`.

---

## 20. CLI REFERANSI

### 20.1 Komutlar (v1 + eklemeler)

```bash
ribozym run <spec_file> [--mode interactive|batch] [--autonomy 1|2|3] [--dry-run]
ribozym status [<spec_id>]
ribozym logs <spec_id> [--follow]
ribozym attach <spec_id>
ribozym resume <spec_id>
ribozym report <spec_id> [--format md|json]
ribozym approve <spec_id> <gate:spec|task_plan|delivery> [--note "..."]   # YENİ (§9)
ribozym models list | blacklist <id> --reason "..." | unblacklist <id>
ribozym calibrate [--suite quick|full]    # YENİ (v2.41): A10 kalibrasyonu — A4 benchmarkları + A7 izole koşum + görev-bazlı tier list üretimi (§5.15)
ribozym budget show [--monthly]
ribozym config validate
ribozym schema validate <file>            # YENİ: herhangi bir run dosyasını şemaya karşı doğrula
ribozym new-form                          # YENİ (v2.5): boş spec_form.md şablonu üretir (§5.4)
ribozym interview                         # YENİ (v2.9): yönlendirilmiş soru-cevap turu → spec_form.md'yi doldurur (§5.4 Aşama 1)
ribozym review <spec_id> decisions        # YENİ (v2.13): karar kartları onay ekranı — Evet/Hayır + serbest yazı (§5.4B)
ribozym suggest <spec_id> "<idea>"        # YENİ (v2.7): kullanıcı öneri kanalı — danışma modeli (§5.5)
```

`--dry-run`: A1 + A2 çalışır (spec + görev ağacı üretir, planı gösterir), **hiçbir araç/bulut çağrısı yapmaz** — maliyetsiz plan önizleme.

### 20.2 Çıkış kodları (yeni sözleşme)

| Kod | Anlam |
| --- | --- |
| 0 | Delivered / başarı |
| 1 | Kullanıcı hatası (hatalı argüman, spec dosyası yok) |
| 2 | Spec reddedildi / NeedsUser |
| 3 | `BUDGET_EXCEEDED` |
| 4 | Onay zaman aşımı / ret |
| 5 | Beklenmedik çökme / iç hata |

---

## 21. TEST STRATEJİSİ (v1'de tek satırdı)

### 21.1 İlke: determinizm

Testlerde LLM **asla** canlı çağrılmaz. İki sahte (mock) düzeyi:

- `wiremock` + `MockLLM`: sabit dosyalardan (JSONL) sırayla yanıt — her şema-dışı yanıt, zaman aşımı, 429 senaryosu bir dosyadır.
- `insta` golden test: `ExecutionResult`, rapor, olay kaydı çıktıları referans dosyayla karşılaştırılır.

### 21.2 Katmanlar

| Katman | Kapsam | Örnek |
| --- | --- | --- |
| Birim | seçim formülü, bütçe kelepçe matematiği, DAG zamanlayıcı (döngü, dosya-sahipliği sıralaması), izin listesi eşleştirici, resume ledger kurtarma | kontrol düzleminde %60+ kapsam hedefi |
| Bütünleşik | FSM geçiş tablosu birebir; olay zarfı `seq` artan; SQLite migration'lar | tmpdir + geçici DB |
| Uçtan uca golden | `examples/spec_rest_api.md` + MockLLM → tam run → `StandardDepartmentReport.json` bayt-karşılaştırma; **resume varyantı:** run ortasında `kill`, resume, aynı nihai hash; **ayrık-çökme varyantı (v2.47 B4):** ledger/olay yazımı arasına enjekte edilen çökme noktaları (4 yazım adımının her biri sonrası) → resume aynı nihai hash | CI'da zorunlu adım |
| Fuzz/özellik | izin listesi çözümleyicisine argv fuzz (zamanlayıcıya rastgele DAG üretimi) | `proptest` |

### 21.3 Kabul eşikleri (CI adımları — Q6 kararı: Linux-önce + Windows uyumu)

- **Ana matris (`ubuntu-latest`):** `cargo fmt --check` · `clippy -D warnings` · `cargo test` · `cargo deny check` · `cargo audit` (haftalık) · golden e2e + resume-e2e · kontrol düzlemi kapsamı ≥ %70.
- **Windows uyum işi (`windows-latest`):** `cargo check` + `cargo test` — her push'ta koşar.
- Kod **platform-bağımsız** yazılır: yol işlemleri argv-biçimi + normalize (§12.2); sabit kodlanmış `/` veya `\` yasaktır (clippy lint ile zorlanır).

---

## 22. TESLİM PLANI — FAZLAR, ÇIKIŞ KRİTERLERİ, MVP KESİMİ (v2.42: 3 günlük sprint)

**Takvim kararı (kullanıcı):** planlama 2026-09-12'de tamamlandı; kodlama **bugün başlar, en fazla 2–3 günde biter** (LLM-destekli geliştirme). Fazlar sıralıdır; her fazın çıkış kriteri **çalışan bir testte kanıtlanır**; tahminler saat ölçeğindedir (8 saat = 1 gün). 13 günlük kalibrasyon penceresi (§5.15) bolca yeterlidir.

| Gün | Faz | Kapsam | Çıkış kriteri (kanıt) | Tahmin |
| --- | --- | --- | --- | --- |
| 1 | 0 | İskelet: cargo workspace, CI, `.gitignore`, `.env.example`, `schemas/` iskeleti | CI yeşil, boş `config validate` çalışır | 2–3 sa |
| 1 | 1 | Modeller + SQLite (task_scores dahil) + olay zarfı + ledger | Şema testleri + `seq` artanlık testi + migration 0001 | 3–4 sa |
| 1 | 2 | Çıkarım (yerel/bulut), ortam katmanı (tekrar/sınırlandırıcı/loglama), **MockLLM altyapısı** | wiremock üzerinde `ModelSchemaInvalid→repair` testi | 3–4 sa |
| 1 | 3 | `Agent` arayüzü, `AgentContext`, ToolGateway (normalize→izin listesi→izolasyon) | argv-fuzz + izin listesi ret testleri; `TOOLCHAIN_MISSING` ön-kontrolü | 3 sa |
| 1 son | **calibrate (hızlı)** | A10 kalibrasyonu — A4 benchmark seti + 6 API'nin A7-izole koşumu (pencere aktif) | 6 API'nin ilk tier list'i yazılmış, havuz dolu | 2 sa |
| 2 | 4 | Orkestratör: FSM + DAG zamanlayıcı + resume + kapılar + bütçe kelepçesi | golden mini-run (3 sahte görev, paralel grup, kill→resume aynı hash) | 6–8 sa |
| 2 | 5 | Ajanlar — **MVP kümesi: A1, A2, A5, A7, A4, A3** (+ A10 çekirdeği) | `examples/spec_rest_api.md` uçtan uca Delivered (yalnız Rust, bulut+yerel) | 6–8 sa |
| 3 | 6 | A8, A9 (doküman + git + dağıtım yüzeyi, push kapısı) | üretilen proje kendi CI'sında yeşil; kurulum yolu A7 izolinde sıfırdan doğrulanmış | 4 sa |
| 3 | 7-lite | A6, A11 (agent-reach birincil, DDG yedek; **doğrulama yalnız kural katmanı**), enjeksiyon filtresi | web önbelleği + enjeksiyon filtre testleri | 4 sa |
| 3 | 8 | A10 tam: OpenRouter+AA aday eleme + benchmark tier list + kara liste otomasyonu | `calibrate` tam koşumu, görev-bazlı tier list'ler dolu | 4 sa |

**MVP kesim çizgisi:** v0.1 = **tüm ajanlar (A1–A12)**, gün 1–3'ün tamamı; web katmanı bilinçli olarak basitleştirilmiştir (agent-reach arama + kural tabanlı doğrulama + enjeksiyon filtresi). v1.0 = + 7-tam (LLM kör puanlama, Tavily, arama↔doğrulama uyuşmazlık izleme) + 8b (`prompt_optimizer` gerçek algoritmaları, Python/TS desteği). Gerekçe: kullanıcı kararı — ajan kadrosu baştan eksiksiz olsun, belirsizlik parçalara bölünmüş kalsın.

---

## 23. RİSK KAYDI

| # | Risk | İ | E | Azaltım |
| --- | --- | --- | --- | --- |
| R1 | LLM şemaya uymayan/yanlış görev ağacı üretir → zamanlayıcı kilitlenir | O | Y | Şema kontrolü + düzeltme turu; döngü tespiti; son çare NeedsUser (§16) |
| R2 | Bütçe sızıntısı (tekrar deneme patlaması) | O | O | Ön-kelepçe (§8.2) + E0 bekleme + aylık sert sınır |
| R3 | A5 kodu gereksinimi değil "testi geçmeyi" hedefler | O | Y | Kabul kriterleri A4'ün ürettiği bağımsız testlerle ölçülür; A5'e test dosyası yazma yasağı (sahiplik §6.2) |
| R4 | Web içeriğiyle prompt enjeksiyonu | O | Y | A11 veri/talimat işaretleme; izole ortamda LLM komut önerisi izin listesine çarpar |
| R5 | Resume'da yarım `git commit` | D | O | git_op ledger + SHA doğrulama (§7.4) |
| R6 | VRAM izleme planlandığı gibi çalışmaz (`sysinfo`) | — | — | **Önemsizleşti:** §4 nvml-wrapper |
| R7 | OpenRouter fiyat/model değişkenliği önbelleği bayatlatır | O | O | Fiyat çekimi 5 dk'da bir (§8.2 m5); alınamazsa son bilinen fiyat + %15 pay (`PRICE_STALE`, §5.2); model silinirse → `MODEL_UNAVAILABLE` → sıralamadaki yedeğe düş |
| R8 | Windows'ta izolasyon/izin listesi zayıflığı (shell yok ama yol ayracı) | O | O | argv-biçimi zorunlu; yol normalize testleri win/linux CI matrisinde |
| R9 | A11 doğrulamasının önyargısı aramayla korele (aynı/aile modeli) | O | O | Dışlama **model kimliği değil aile bazlı** yapılandırılabilir (config); çağrı-bazlı kanıt (§10.3) |
| R10 | Tek-kopya kilidi tutarsız (çökme → ölü kilit) | D | O | pid+değişim-zamanı ile geri alma (§7.5) |
| R11 | Arşiv bölümü sessizce bozulur → sonraki aşama yanlış bağlamla koşar (bağlamın tek kaynağı arşiv) | O | O | sha256 manifest + okuma-anı doğrulama; bozuk bölümde sessiz-devam yasak — yeniden-yazım ya da NeedsUser (§7.6, v2.47 B2) |
| R12 | Run %90 ilerlemişken bütçe tükenir → Failed tüm işi kullanılamaz kılar; fiyat-lehine kayma sessiz kalite düşüşü yaratır | O | O | Bütçe duvarı `Paused(budget)` + resume/kapsam-küçültme yolu; kalite takası `quality_tradeoff` alanıyla rapora yazılır (§8.2, v2.47 B3) |

---

## 24. KESİN KURALLAR (v1'in 10 maddesi — her biri makineyle doğrulanabilir hâle getirildi)

1. **İngilizce** → lint: `schemas/` + `config/prompts/` içinde Türkçe karakter taraması (CI).
2. **Kodda gizli anahtar yok** → A7 adımı: `SEC_SECRET_FOUND` run'ı durdurur (§12.5).
3. **Resume** → golden test: "ortada kill, resume, aynı nihai hash" (§21.2).
4. **Olay güdümlü** → her durum değişimi ledger + `events.jsonl`'a yazılır; veritabanı yazımı olmayan geçiş imkânsızdır (yazım kontrol düzleminde toplanmıştır).
5. **API-önce** → `lib.rs`'in herkese açık yüzeyi (§3.3); CLI'ın veritabanına doğrudan yazması yasaktır.
6. **Bütçe** → §8.2 kontrol noktaları; ön-kelepçesiz bulut çağrısı API'de imkânsızdır (ortam katmanı zorunlu).
7. **Güven ama doğrula** → doğrulama girdisi (`VerificationRequest`) üretici alanı taşımaz (tip düzeyinde körlük, §13.3).
8. **İzin listesi** → argv normalizasyonu + bilinmeyen bayrak reddi (§12.2).
9. **Önce en ucuz** → model seçimi A10'dadır: eşdeğer kalitede daha ucuz aday varsa o seçilir; bütçe baskısında seçim fiyata kayar (§8.2, §10.3); güçlü model yalnız zorluk analizinin gerekçesiyle (birim test: eskalasyon sayacı).
10. **Çift imza** → `Delivered` olayı üç koşula bağlıdır: rapor + `ComplianceCheck.approved` + `approvals[delivery]`.
11. **AI danışmandır, kullanıcı karar vericidir** → her öneri analiz edilir ve cevabı gerekçesiyle kullanıcıya döner; **işe koşma yalnız kullanıcı onayıyla** olur (§5.5).
12. **Çıktı ekonomisi** → her LLM çıktısı yapılandırılmış bir dosyadır; tek üretim çok kullanım; tekrarlayan kalıplar koddan şablonla doldurulur; büyük girdi + küçük çıktı ilkesi prompt'ların son talimatıdır (§14.1).
13. **Ajan iletişimi tek zarftadır** → tüm ajan-ajansı mesajlar JSON zarfla Rust çekirdeğinden taşınır; sabit alanlar koddan, LLM yalnız üretilmesi gereken payload alanlarını yazar; serbest sohbet yok, tüm iletişim izlenebilirdir (§11.9).

---

## 25. KARAR KAYDI (eski "açık sorular" — v2.1'de tamamı kapatıldı)

| # | Karar | Belgeye işlendi |
| --- | --- | --- |
| Q1 | **Bulut API birincil motor; yerel model yalnız yardımcı yük paylaşımı** (tek başına yeterli değil; performans/maliyet hızlandırıcısı). Yerel model: Qwen2.5-Coder-7B-Instruct Q4_K_M (doğrulandı: `C:\Users\Rog\qwen_model\`, RTX 5050 8 GB). v2.3'te netleştirildi | §10.1, §10.2 |
| Q2 | **Tüm ajanlar MVP'de (A1–A12; doğrulama v2.45'te A11'e birleşti).** A11 basitleştirilmiş (agent-reach birincil + DDG yedek), doğrulama MVP'de yalnız kural tabanlı (LLM kör puanlama v1.0'da), enjeksiyon filtresi MVP'de | §13, §22 |
| Q3 | Mevcut projeye çalışma: **izole çalışma alanı** (tercihen konteyner) — orijinale asla doğrudan yazılmaz; mevcut testler geriye-dönük kapı; teslim patch/PR + onay | §12.6 |
| Q4 | Onay **üretimi durdurmaz**: düşük riskli kapılar (spec, task_plan) zaman aşımında otomatik onay + kayda geçme; yalnız `delivery` insan bekler (park + resume) | §9 |
| Q5 | **agent-reach birincil arama aracı** (v0.1'den itibaren), DDG yedek | §13.1 |
| Q6 | **Linux-önce CI + Windows uyum testi**: ana matris ubuntu-latest, ayrı windows-latest işi (check+test); kod platform-bağımsız | §21.3 |

---

## EK A — OLAY KATALOGU (v1'e eklenenler ★)

Lifecycle: `SpecReceived, SpecValidated, SpecRejected, SpecRevised` · Planning: `TasksCreated, TasksRevised, ★TaskGraphRejected` · Execution: `TaskStarted, TaskCompleted, TaskFailed, TaskRetried, TaskSkipped, TaskBlocked, ★TaskOwnershipConflict` · Escalation: `EscalatedToCEO, CrisisResolved, EscalatedToCloud, ★EscalatedToUser` · Human: ★`GateRequested, GateTimeout,` + `HumanApproval*` · ★User ideas: `UserIdeaProposed, UserIdeaIntegrated, UserIdeaRejected` · Delivery: `Delivered, DeliveryRejected` · Budget: `BudgetWarning, BudgetExceeded, ★MonthlyBudgetExceeded` · Model: `ModelSelected, ModelBlacklisted, NewModelDiscovered, ★ModelRepairTried` · Web: `WebSearchRequested, WebSearchCompleted, VerificationScored, ★WebContentFlaggedInjection` · System: ★`PreflightFailed, SandboxViolation, LockStolen`

## EK B — ENUM DEĞERLERİ

v1'deki tüm enum değerleri aynen geçerlidir (`mode`, `priority`, `target`, görev `status`, `escalation_level` 0–4 merdivenine bağlandı, `final_status`, `confidence high|medium|low` eşikleri §13.3'te). Yenileri: `ComplianceCheck.verdict: approved|needs_revision|rejected` · `gate: spec|task_plan|delivery` · `model_tier: low|medium|high|none` · `WebFinding.source.type: docs|gov|edu|news|blog|repo|other`.

---

*Bu belge v2.2 — uygulamanın (kodun) referans sözleşmesidir. Değişiklikler versiyon numarası + diff özeti ile izlenir; §25'teki karar kaydı tamamlandığı için Faz 0'a başlanabilir.*

## 26. İSİM VE GÖRÜNÜRLÜK KARARI (2026-09-11)

- **Ürün adı: `ribozym`** (paket + binary aynı). Metafor: ribozom/ribozym, mRNA'yı (spec) okuyup proteine (yazılım) çeviren hücresel makine — sistemin işinin birebir biyolojik karşılığı.
- **crates.io:** `ribozym` boş (`anabol` da boş; `ribosome`, `mycelium`, `morpho`, `axolotl`, `chrysalis`, `ribo` alınmış).
- **GitHub görünürlük:** `ribozym` neredeyse hiç kullanılmamış (max 2★, alakasız küçük depolar); `mycelium` kalabalık (800★ OS, 712★ cüzdan, 117★ multi-agent projesi), `ribosome` 251★'lık bir code-generator ile kavram çakışmasında.
- Repo kökü: `C:\Users\Rog\Desktop\ribozym\`.
