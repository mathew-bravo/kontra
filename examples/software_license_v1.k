-- Software License Agreement demo (Phase 2 v1 baseline)
contract SoftwareLicense {
  parties {
    licensor: "Acme Corp"
    licensee: "Beta Inc."
  }

  event Effective = date("2026-03-01")
  event Delivery = triggered_by(licensor)
  event AcceptanceNotice = triggered_by(licensee)

  term DeliveryPeriod = 30 business_days from Effective
  term PaymentWindow = 15 calendar_days from AcceptanceNotice
  term CurePeriod = 10 business_days from breach_of(DeliverSoftware)

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

  remedy CureOrTerminate on breach_of(DeliverSoftware) {
    phase Cure {
      party: licensor
      action: "Deliver software within cure period"
      due: CurePeriod
      condition: after(Effective)
    }
    phase Terminate on breach_of(Cure) {
      action: "Licensor refunds any amounts paid"
    }
  }
}
