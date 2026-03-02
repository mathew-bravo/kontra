-- Software License Agreement demo (Phase 2 v2 revised)
contract SoftwareLicense {
  parties {
    licensor: "Acme Corp"
    licensee: "Beta Inc."
  }

  event Effective = date("2026-03-01")
  event Delivery = triggered_by(licensor)
  event AcceptanceNotice = triggered_by(licensee)

  term DeliveryPeriod = 45 business_days from Effective
  term PaymentWindow = 15 calendar_days from AcceptanceNotice

  obligation DeliverSoftware {
    party: licensor
    action: "Deliver software per Schedule A"
    due: DeliveryPeriod
    condition: after(Effective)
  }

  obligation PayLicenseFee {
    party: licensee
    action: "Pay $50,000 license fee"
    due: PaymentWindow
    condition: occurred(AcceptanceNotice) and satisfied(DeliverSoftware)
  }

  obligation AuditRights {
    party: licensee
    action: "Licensee may audit licensor annually"
  }
}

