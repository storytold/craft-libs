//! The operation-level client: one method per WS-Print operation, over any [`Transport`].

use super::adapter::{Attachment, Call, SoapClient, Transport};
use super::error::{REPLY_LABEL, Result, WsdError};
use super::model::{JobElements, JobStatus, JobSummary, PrinterElements, Section};
use super::mtom::{self, Transfer};
use super::ns::{self, Action};
use super::ops::{self, JobSection};
use super::policy::{NetworkLimits, PrinterUrl};
use super::ticket::{DocumentDescription, PrintTicket};

/// A client bound to one printer: the print operations over a generic [`SoapClient`].
pub struct Client<T: Transport> {
    soap: SoapClient<T>,
    url: PrinterUrl,
}

/// The generic call for a print operation with this body.
fn call_for<'a>(action: Action, body: &str) -> Call<'a> {
    Call::new(action.uri(), action.response_uri())
        .reply_element(ns::WPRT, format!("{}Response", action.name()))
        .namespace("wprt", ns::WPRT)
        .body(body)
}

impl<T: Transport> Client<T> {
    pub fn new(transport: T, url: PrinterUrl) -> Self {
        let soap = SoapClient::new(transport, url.as_device().clone()).with_reply_label(REPLY_LABEL);
        Self { soap, url }
    }

    pub fn with_limits(mut self, limits: NetworkLimits) -> Self {
        self.soap = self.soap.with_limits(limits);
        self
    }

    pub fn url(&self) -> &PrinterUrl {
        &self.url
    }

    pub fn limits(&self) -> NetworkLimits {
        self.soap.limits()
    }

    /// The generic SOAP client underneath (to add a deadline or cancel token, for instance).
    pub fn soap(&self) -> &SoapClient<T> {
        &self.soap
    }

    fn call<R>(&self, action: Action, body: &str, read: impl FnOnce(roxmltree::Node<'_, '_>) -> Result<R>) -> Result<R> {
        self.soap.call(&call_for(action, body), read)
    }

    /// GetPrinterElements for the given sections.
    pub fn get_printer_elements(&self, sections: &[Section]) -> Result<PrinterElements> {
        self.call(Action::GetPrinterElements, &ops::get_printer_elements_body(sections), ops::parse_printer_elements)
    }

    /// CreatePrintJob: the job id.
    pub fn create_print_job(&self, ticket: &PrintTicket) -> Result<u32> {
        self.call(Action::CreatePrintJob, &ops::create_print_job_body(ticket)?, ops::parse_create_print_job)
    }

    /// SendDocument with the bytes inline or as an MTOM part.
    pub fn send_document(
        &self,
        job_id: u32,
        description: &DocumentDescription,
        ticket: &PrintTicket,
        last_document: bool,
        data: &[u8],
        transfer: Transfer,
    ) -> Result<()> {
        let action = Action::SendDocument;
        let dp = ticket.document_processing_xml();
        let cid = format!("doc.{}@craft-print", super::adapter::new_uuid());
        let limits = self.soap.limits();
        let data_xml = match transfer {
            Transfer::Mtom => mtom::xop_include(&cid),
            Transfer::InlineBase64 => {
                if data.len() > limits.max_request / 2 {
                    return Err(WsdError::TooLarge("an inline (base64) document", limits.max_request / 2));
                }
                mtom::inline_data(data)
            }
        };
        let body = ops::send_document_body(job_id, description, &dp, last_document, &data_xml)?;
        let call = call_for(action, &body);
        let call = if transfer == Transfer::Mtom { call.attachment(Attachment { cid: &cid, data }) } else { call };
        self.soap.call(&call, |_| Ok(()))
    }

    /// CancelJob.
    pub fn cancel_job(&self, job_id: u32) -> Result<()> {
        self.call(Action::CancelJob, &ops::cancel_job_body(job_id)?, |_| Ok(()))
    }

    /// GetJobElements for the given sections of one job.
    pub fn get_job_elements(&self, job_id: u32, sections: &[JobSection]) -> Result<JobElements> {
        self.call(Action::GetJobElements, &ops::get_job_elements_body(job_id, sections)?, ops::parse_job_elements)
    }

    /// The job's status; a printer that does not return one is a protocol error.
    pub fn job_status(&self, job_id: u32) -> Result<JobStatus> {
        let elements = self.get_job_elements(job_id, &[JobSection::Status])?;
        elements.status.ok_or_else(|| WsdError::Protocol("the printer returned no JobStatus".into()))
    }

    /// GetActiveJobs.
    pub fn get_active_jobs(&self) -> Result<Vec<JobSummary>> {
        self.call(Action::GetActiveJobs, &ops::get_active_jobs_body(), |n| ops::parse_job_list(n, "ActiveJobs"))
    }

    /// GetJobHistory.
    pub fn get_job_history(&self) -> Result<Vec<JobSummary>> {
        self.call(Action::GetJobHistory, &ops::get_job_history_body(), |n| ops::parse_job_list(n, "JobHistory"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wsd::adapter::{DeviceUrl, Request, Response, WireError};
    use crate::wsd::testutil::reply;

    struct Canned(u16, Option<&'static str>, Vec<u8>);

    impl Transport for Canned {
        fn post(&self, _: &DeviceUrl, _: Request) -> Result<Response, WireError> {
            Ok(Response::new(self.0, self.1.map(str::to_owned), self.2.clone()))
        }
    }

    fn client(status: u16, ct: Option<&'static str>, body: impl Into<Vec<u8>>) -> Client<Canned> {
        Client::new(Canned(status, ct, body.into()), PrinterUrl::parse("http://127.0.0.1:5357/x").unwrap())
    }

    fn err(c: &Client<Canned>) -> WsdError {
        c.get_printer_elements(&[Section::Status]).unwrap_err()
    }

    #[test]
    fn a_non_success_reply_that_is_not_a_soap_fault_is_an_http_error() {
        let big_html = format!("<html>{}</html>", "<p>x</p>".repeat(2_000_000));
        let soap_no_fault = reply("urn:x", None, "<wprt:GetPrinterElementsResponse/>");
        let cases: Vec<(u16, Option<&'static str>, Vec<u8>)> = vec![
            (503, Some("text/html"), b"<html><body>maintenance</body></html>".to_vec()),
            (
                503,
                Some("text/html"),
                b"  
<!DOCTYPE html><html><body>bad</body>"
                    .to_vec(),
            ),
            (500, Some("application/xml"), b"<error>boom</error>".to_vec()),
            (500, None, soap_no_fault.into_bytes()),
            (503, Some("text/html"), big_html.into_bytes()),
            (502, Some("text/html"), vec![b'<', 0, 159, 146, 150, 0xff, 0xfe]),
            (500, None, b"<".to_vec()),
            (503, None, Vec::new()),
            (404, Some("text/plain"), b"not found".to_vec()),
            (500, Some("multipart/related; boundary=b"), b"<html>".to_vec()),
        ];
        for (status, ct, body) in cases {
            let e = err(&client(status, ct, body));
            assert_eq!(e, WsdError::Http { status, retry_after: None }, "{status} {ct:?}");
        }
    }

    #[test]
    fn retryability_follows_the_status() {
        for (status, retryable) in [
            (500, true),
            (502, true),
            (503, true),
            (504, true),
            (429, true),
            (408, true),
            (425, true),
            (501, false),
            (505, false),
            (400, false),
            (401, false),
            (404, false),
        ] {
            let e = err(&client(status, Some("text/html"), "<html>x</html>"));
            assert_eq!((e.kind(), e.is_retryable()), ("http", retryable), "{status}");
        }
    }

    #[test]
    fn a_soap_fault_in_a_non_success_reply_is_still_a_fault() {
        let fault = "<s:Fault><s:Code><s:Value>s:Receiver</s:Value><s:Subcode><s:Value xmlns:wprt=\"http://schemas.microsoft.com/windows/2006/08/wdp/print\">wprt:ServerErrorTemporaryError</s:Value></s:Subcode></s:Code><s:Reason><s:Text xml:lang=\"en\">busy</s:Text></s:Reason></s:Fault>";
        let e = err(&client(500, Some("application/soap+xml"), reply("http://www.w3.org/2005/08/addressing/fault", None, fault)));
        assert!(matches!(e, WsdError::Fault(ref f) if f.code.is_retryable()), "{e:?}");
        assert!(e.is_retryable());
    }

    #[test]
    fn an_oversized_success_reply_is_still_too_large() {
        let c = client(200, Some("text/xml"), vec![b' '; NetworkLimits::default().max_response + 1]);
        assert!(matches!(err(&c), WsdError::TooLarge(..)));
    }
}
