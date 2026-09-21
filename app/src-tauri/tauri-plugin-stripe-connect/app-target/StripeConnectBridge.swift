import StripeConnect
import UIKit

// Compiled into the APP target (the app's build script adds this directory to
// its sources and links StripeConnect via SwiftPM), not into the plugin's
// static library — see the plugin's src/lib.rs. StripeConnectPlugin.swift
// finds this class by its ObjC name and calls presentWithRequest: with:
//
//   publishableKey         String
//   presenter              UIViewController
//   fetchSecret            block (block (NSString?) -> Void) -> Void
//   onExit                 block (NSString? error) -> Void
//   privacyPolicyUrl       String, optional
//   fullTermsOfServiceUrl  String, optional
//
// The block signatures must match the typealiases in StripeConnectPlugin.swift.

private typealias SecretCompletion = @convention(block) (NSString?) -> Void
private typealias FetchSecretBlock = @convention(block) (@escaping SecretCompletion) -> Void
private typealias ExitBlock = @convention(block) (NSString?) -> Void

@objc(TauriStripeConnectBridge)
final class TauriStripeConnectBridge: NSObject, AccountOnboardingControllerDelegate {
    /// Keeps the session alive while onboarding is on screen; the SDK only
    /// weakly references its delegate.
    private static var active: TauriStripeConnectBridge?

    private let componentManager: EmbeddedComponentManager
    private let onExit: ExitBlock
    private var loadError: String?

    private init(componentManager: EmbeddedComponentManager, onExit: @escaping ExitBlock) {
        self.componentManager = componentManager
        self.onExit = onExit
    }

    @objc(presentWithRequest:)
    static func present(request: NSDictionary) {
        guard let publishableKey = request["publishableKey"] as? String,
              let presenter = request["presenter"] as? UIViewController,
              let fetchSecretObject = request["fetchSecret"],
              let onExitObject = request["onExit"] else {
            return
        }
        let fetchSecret = unsafeBitCast(fetchSecretObject as AnyObject, to: FetchSecretBlock.self)
        let onExit = unsafeBitCast(onExitObject as AnyObject, to: ExitBlock.self)

        // A second call while one is still open would stack two sheets.
        if let previous = active {
            previous.finish()
        }

        let componentManager = EmbeddedComponentManager(
            apiClient: STPAPIClient(publishableKey: publishableKey),
            appearance: appearance(),
            fetchClientSecret: {
                await withCheckedContinuation { continuation in
                    fetchSecret { secret in
                        continuation.resume(returning: secret as String?)
                    }
                }
            }
        )

        let bridge = TauriStripeConnectBridge(componentManager: componentManager, onExit: onExit)
        active = bridge

        let controller = componentManager.createAccountOnboardingController(
            fullTermsOfServiceUrl: (request["fullTermsOfServiceUrl"] as? String).flatMap(URL.init(string:)),
            privacyPolicyUrl: (request["privacyPolicyUrl"] as? String).flatMap(URL.init(string:))
        )
        controller.delegate = bridge
        controller.present(from: presenter)
    }

    /// GetPayed's palette, so the sheet reads as part of the app rather than
    /// a web page dropped on top of it.
    private static func appearance() -> EmbeddedComponentManager.Appearance {
        var appearance = EmbeddedComponentManager.Appearance.default
        appearance.colors.primary = UIColor(red: 0x2E / 255, green: 0x5E / 255, blue: 0x4E / 255, alpha: 1) // evergreen
        appearance.colors.text = UIColor(red: 0x1F / 255, green: 0x29 / 255, blue: 0x33 / 255, alpha: 1)    // midnight slate
        appearance.colors.background = UIColor(red: 0xF7 / 255, green: 0xF9 / 255, blue: 0xFA / 255, alpha: 1) // glacier white
        return appearance
    }

    private func finish() {
        onExit(loadError as NSString?)
        if TauriStripeConnectBridge.active === self {
            TauriStripeConnectBridge.active = nil
        }
    }

    // MARK: AccountOnboardingControllerDelegate

    func accountOnboardingDidExit(_ accountOnboarding: AccountOnboardingController) {
        finish()
    }

    func accountOnboarding(_ accountOnboarding: AccountOnboardingController,
                           didFailLoadWithError error: Error) {
        // The SDK shows its own error screen; report it once the user closes it.
        loadError = error.localizedDescription
    }
}
