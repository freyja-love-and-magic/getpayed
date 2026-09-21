import Tauri
import UIKit

// The Stripe SDK isn't linked into this package (see src/lib.rs). The app
// target compiles app-target/StripeConnectBridge.swift, which does import it;
// we reach that class by name and talk to it through ObjC blocks, since the
// two modules can't see each other's types.

typealias SecretCompletion = @convention(block) (NSString?) -> Void
typealias FetchSecretBlock = @convention(block) (@escaping SecretCompletion) -> Void
typealias ExitBlock = @convention(block) (NSString?) -> Void

let bridgeClassName = "TauriStripeConnectBridge"
let bridgeSelector = NSSelectorFromString("presentWithRequest:")

struct PresentOnboardingArgs: Decodable {
    let publishableKey: String
    let clientSecret: String
    /// Pinged when the SDK needs a fresh client secret (the session expired
    /// mid-onboarding). JS answers with provide_client_secret.
    let onRefresh: Channel
    let privacyPolicyUrl: String?
    let fullTermsOfServiceUrl: String?
}

struct ProvideClientSecretArgs: Decodable {
    let clientSecret: String?
}

struct RefreshRequest: Encodable {
    let reason = "expired"
}

@objc public class StripeConnectPlugin: Plugin {
    /// The SDK's outstanding request for a client secret, if any. Only one at
    /// a time: a newer request answers the older one with nil.
    private var pendingSecret: SecretCompletion?

    @objc public func presentOnboarding(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(PresentOnboardingArgs.self)

        guard let bridge = NSClassFromString(bridgeClassName) as? NSObject.Type,
              bridge.responds(to: bridgeSelector) else {
            invoke.reject("Stripe Connect isn't linked into this app build (\(bridgeClassName) not found)")
            return
        }

        // The SDK asks once on load — answered from args, no round trip — and
        // again each time the session expires, which goes out to JS.
        var initialSecret: String? = args.clientSecret
        let fetchSecret: FetchSecretBlock = { [weak self] completion in
            DispatchQueue.main.async {
                if let secret = initialSecret {
                    initialSecret = nil
                    completion(secret as NSString)
                    return
                }
                guard let self = self else { completion(nil); return }
                self.pendingSecret?(nil)
                self.pendingSecret = completion
                do {
                    try args.onRefresh.send(RefreshRequest())
                } catch {
                    self.pendingSecret = nil
                    completion(nil)
                }
            }
        }

        // Resolves (never rejects) once the user leaves onboarding, carrying
        // any load error the SDK reported along the way. Rejecting on a load
        // error would settle the call while the sheet is still on screen.
        var settled = false
        let onExit: ExitBlock = { [weak self] error in
            DispatchQueue.main.async {
                guard !settled else { return }
                settled = true
                self?.pendingSecret?(nil)
                self?.pendingSecret = nil
                if let error = error {
                    invoke.resolve(["error": error as String])
                } else {
                    invoke.resolve([:])
                }
            }
        }

        DispatchQueue.main.async {
            guard let presenter = self.manager.viewController else {
                invoke.reject("No view controller to present onboarding from")
                return
            }
            let request = NSMutableDictionary()
            request["publishableKey"] = args.publishableKey
            request["presenter"] = presenter
            request["fetchSecret"] = fetchSecret
            request["onExit"] = onExit
            if let url = args.privacyPolicyUrl { request["privacyPolicyUrl"] = url }
            if let url = args.fullTermsOfServiceUrl { request["fullTermsOfServiceUrl"] = url }
            _ = bridge.perform(bridgeSelector, with: request)
        }
    }

    @objc public func provideClientSecret(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(ProvideClientSecretArgs.self)
        DispatchQueue.main.async {
            let completion = self.pendingSecret
            self.pendingSecret = nil
            completion?(args.clientSecret as NSString?)
            invoke.resolve()
        }
    }
}

@_cdecl("init_plugin_stripe_connect")
func initPlugin() -> Plugin {
    return StripeConnectPlugin()
}
