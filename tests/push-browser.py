# Notifiche push nel browser (Chromium): iscrizione dal pulsante e service worker, con un messaggio push simulato
# tramite gli strumenti per sviluppatori di Chrome. Richiede un sito con notifiche attive servito su http://127.0.0.1:8414.
import asyncio, json, shutil
from playwright.async_api import async_playwright
S = "http://127.0.0.1:8414"
async def main():
    shutil.rmtree("/tmp/chrome-profilo", ignore_errors=True)
    async with async_playwright() as p:
        ctx = await p.chromium.launch_persistent_context("/tmp/chrome-profilo", headless=True, args=["--headless=new"], locale="it-IT")
        await ctx.grant_permissions(["notifications"], origin=S)
        pg = await ctx.new_page(); logs = []; pg.on("console", lambda m: logs.append(m.text))
        await pg.goto(S + "/frana/"); await pg.wait_for_timeout(300)
        await pg.click("[data-push]"); await pg.wait_for_timeout(8000)
        print("1. profilo normale, messaggio sul pulsante:", await pg.inner_text("[data-push] span"))
        print("   dettaglio per chi sviluppa:", logs[:1] or "nessuno")
        sw = ctx.service_workers[0] if ctx.service_workers else await ctx.wait_for_event("serviceworker")
        await sw.evaluate("""() => { self.__calls = []; const orig = self.registration.showNotification.bind(self.registration);
            self.registration.showNotification = (t, o) => { self.__calls.push({title: t, body: o.body, url: o.data && o.data.url}); return orig(t, o).then(() => self.__calls.push({mostrata: true}), e => self.__calls.push({errore: String(e)})); }; }""")
        cdp = await ctx.new_cdp_session(pg); regs = []
        cdp.on("ServiceWorker.workerRegistrationUpdated", lambda e: regs.extend(e["registrations"]))
        await cdp.send("ServiceWorker.enable"); await pg.wait_for_timeout(500)
        rid = next(r["registrationId"] for r in regs if r["scopeURL"].startswith(S))
        await cdp.send("ServiceWorker.deliverPushMessage", {"origin": S, "registrationId": rid, "data": json.dumps({"title": "Il Corriere di prova", "body": "Ultim'ora: riaperta la statale", "url": S + "/frana/"})})
        await pg.wait_for_timeout(1500)
        print("2. service worker:", await sw.evaluate("() => self.__calls"))
        print("3. notifiche visibili:", await pg.evaluate("navigator.serviceWorker.ready.then(r => r.getNotifications()).then(l => l.map(n => ({titolo: n.title, testo: n.body, apre: n.data.url})))"))
        await ctx.close()
asyncio.run(main())
