use k8s_openapi::api::networking::v1::{Ingress, IngressClass};
use kube::api::ListParams;
use kube::{Api, Client, ResourceExt};
use std::collections::HashSet;

/// `spec.controller` of IngressClasses handled by this controller
pub(super) const CONTROLLER_NAME: &str = "networking.kinorca.com/pingress-controller";

const DEFAULT_CLASS_ANNOTATION: &str = "ingressclass.kubernetes.io/is-default-class";
const LEGACY_CLASS_ANNOTATION: &str = "kubernetes.io/ingress.class";

/// IngressClasses whose controller is this controller
pub(super) struct IngressClasses {
    names: HashSet<String>,
    /// One of them is the cluster default class
    is_default: bool,
}

impl IngressClasses {
    pub(super) async fn load(client: Client) -> Result<Self, kube::Error> {
        let api: Api<IngressClass> = Api::all(client);
        let classes = api.list(&ListParams::default()).await?;
        Ok(Self::from_classes(&classes.items))
    }

    fn from_classes(classes: &[IngressClass]) -> Self {
        let ours: Vec<&IngressClass> = classes
            .iter()
            .filter(|c| {
                c.spec
                    .as_ref()
                    .and_then(|s| s.controller.as_deref())
                    .is_some_and(|c| c == CONTROLLER_NAME)
            })
            .collect();

        Self {
            names: ours.iter().map(|c| c.name_any()).collect(),
            is_default: ours.iter().any(|c| {
                c.annotations()
                    .get(DEFAULT_CLASS_ANNOTATION)
                    .is_some_and(|v| v == "true")
            }),
        }
    }

    /// The Ingress is handled by this controller.
    ///
    /// `spec.ingressClassName` is used first, then the deprecated `kubernetes.io/ingress.class`
    /// annotation. An Ingress with neither belongs to the default IngressClass.
    pub(super) fn is_target(&self, ingress: &Ingress) -> bool {
        let class = ingress
            .spec
            .as_ref()
            .and_then(|s| s.ingress_class_name.as_ref())
            .or_else(|| ingress.annotations().get(LEGACY_CLASS_ANNOTATION));

        match class {
            Some(c) => self.names.contains(c),
            None => self.is_default,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::IngressClasses;
    use k8s_openapi::api::networking::v1::{Ingress, IngressClass};
    use serde_json::json;

    fn class(name: &str, controller: &str, default: bool) -> IngressClass {
        serde_json::from_value(json!({
            "metadata": {
                "name": name,
                "annotations": {
                    "ingressclass.kubernetes.io/is-default-class": default.to_string()
                }
            },
            "spec": { "controller": controller }
        }))
        .unwrap()
    }

    fn ingress(class_name: Option<&str>, annotation: Option<&str>) -> Ingress {
        let mut value = json!({ "metadata": { "name": "test" }, "spec": {} });
        if let Some(c) = class_name {
            value["spec"]["ingressClassName"] = json!(c);
        }
        if let Some(a) = annotation {
            value["metadata"]["annotations"] = json!({ "kubernetes.io/ingress.class": a });
        }
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn target_by_class_name() {
        let classes = IngressClasses::from_classes(&[
            class("pingress", super::CONTROLLER_NAME, false),
            class("nginx", "k8s.io/ingress-nginx", true),
        ]);

        assert!(classes.is_target(&ingress(Some("pingress"), None)));
        assert!(!classes.is_target(&ingress(Some("nginx"), None)));
        assert!(!classes.is_target(&ingress(None, None)));
    }

    #[test]
    fn target_by_legacy_annotation() {
        let classes =
            IngressClasses::from_classes(&[class("pingress", super::CONTROLLER_NAME, false)]);

        assert!(classes.is_target(&ingress(None, Some("pingress"))));
        assert!(!classes.is_target(&ingress(None, Some("nginx"))));
        assert!(!classes.is_target(&ingress(Some("nginx"), Some("pingress"))));
    }

    #[test]
    fn target_by_default_class() {
        let classes =
            IngressClasses::from_classes(&[class("pingress", super::CONTROLLER_NAME, true)]);

        assert!(classes.is_target(&ingress(None, None)));
    }
}
