# atar

Ephemeral Terraform runner: applies on start, auto-destroys on exit or failure.

## Usage

Deploy a Terraform configuration and keep it running. The resources
will be destroyed when you press Ctrl+C or when the process exits.

```bash
atar deploy --terraform /path/to/terraform/main.tf \
  --region us-west-2 --instance_type t2.micro
```
After a successful deploy, Terraform output variables are displayed automatically.
